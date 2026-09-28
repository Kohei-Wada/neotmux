use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use neotmux_proto::screen::{Lines, PaneRead, Trailing, Want};
use neotmux_proto::{Dir, Geometry, PaneId, Rect, SplitDir};

use crate::api::rect;
use crate::host::{Op, OpBuffer};
use crate::host::{PaneInfos, PaneTtys};
use crate::plugin::{self, Sources};

/// A count a caller gave in Lua, where the only wrong value is a negative one: that is a
/// typo rather than a refusal, as a resize by a negative number of cells is.
fn count(call: &str, field: &str, value: Option<i64>) -> mlua::Result<Option<u64>> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                mlua::Error::RuntimeError(format!(
                    "pane.{call} takes a {field} of zero or more, got {value}"
                ))
            })
        })
        .transpose()
}

/// A boolean a caller may have left out, which is the same as `false` everywhere here.
fn flag(opts: Option<&Table>, name: &str) -> mlua::Result<bool> {
    match opts {
        Some(opts) => Ok(opts.get::<Option<bool>>(name)?.unwrap_or(false)),
        None => Ok(false),
    }
}

/// What a read of a pane's text was asked to include. `pad` keeps the blank rest of every
/// row rather than cutting it, and `wrapped` asks for the per-row continuation flags;
/// absent, both are off, which is what reading a line of text wants.
fn want(opts: Option<&Table>) -> mlua::Result<Want> {
    Ok(Want {
        trailing: if flag(opts, "pad")? {
            Trailing::Pad
        } else {
            Trailing::Trim
        },
        wrapped: flag(opts, "wrapped")?,
        runs: false,
    })
}

/// A colour as the draw verbs take one, so that a colour read can be drawn back: a palette
/// index, `#rrggbb`, or nil for the terminal's own.
fn color_value(lua: &Lua, color: neotmux_proto::Color) -> mlua::Result<Value> {
    Ok(match color {
        neotmux_proto::Color::Default => Value::Nil,
        neotmux_proto::Color::Idx(i) => Value::Integer(i64::from(i)),
        neotmux_proto::Color::Rgb(r, g, b) => {
            Value::String(lua.create_string(format!("#{r:02x}{g:02x}{b:02x}"))?)
        }
    })
}

/// One row as its runs, each `{ text =, fg =, bg =, bold = true, … }` with an attribute there
/// only where it is on.
fn runs_value(lua: &Lua, rows: Vec<Vec<neotmux_proto::screen::Run>>) -> mlua::Result<Value> {
    let out = lua.create_table()?;
    for (i, row) in rows.into_iter().enumerate() {
        let runs = lua.create_table()?;
        for (j, run) in row.into_iter().enumerate() {
            let entry = lua.create_table()?;
            entry.set("text", run.text)?;
            entry.set("fg", color_value(lua, run.fg)?)?;
            entry.set("bg", color_value(lua, run.bg)?)?;
            for (name, on) in [
                ("bold", run.bold),
                ("dim", run.dim),
                ("italic", run.italic),
                ("underline", run.underline),
                ("inverse", run.inverse),
            ] {
                if on {
                    entry.set(name, true)?;
                }
            }
            runs.set(j + 1, entry)?;
        }
        out.set(i + 1, runs)?;
    }
    Ok(Value::Table(out))
}

/// One read of a pane's text as Lua sees it: the lines, the revision they were read at, and
/// the continuation flags where they were asked for — `nil` where they were not.
fn answer(lua: &Lua, text: Lines) -> mlua::Result<(Value, Value, Value)> {
    let wrapped = match text.wrapped {
        Some(wrapped) => Value::Table(lua.create_sequence_from(wrapped)?),
        None => Value::Nil,
    };
    let rows = match text.runs {
        Some(runs) => runs_value(lua, runs)?,
        None => Value::Table(lua.create_sequence_from(text.lines)?),
    };
    Ok((rows, Value::Integer(text.revision as i64), wrapped))
}

/// The M2 pane API (§6), signatures frozen now. `list`, `current` and `rect` read this
/// frame's geometry; `content`, `revision` and `title` read the pane's own terminal;
/// `split`, `focus` and `resize` are ops on the buffer (§4.6), applied after the tick, so a
/// callback never sees its own request take effect synchronously.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    ops: Rc<OpBuffer>,
    geometry: Rc<RefCell<Geometry>>,
    pane_info: Rc<RefCell<PaneInfos>>,
    pane_tty: Rc<RefCell<PaneTtys>>,
    terminals: Rc<dyn PaneRead>,
) -> mlua::Result<Table> {
    let pane = lua.create_table()?;

    {
        let geometry = geometry.clone();
        let list = lua.create_function(move |lua, ()| {
            let ids: Vec<u32> = geometry
                .borrow()
                .panes
                .iter()
                .map(|(pane, _)| pane.get())
                .collect();
            lua.create_sequence_from(ids)
        })?;
        pane.set("list", list)?;
    }

    {
        let geometry = geometry.clone();
        let current = lua.create_function(move |_, ()| {
            Ok(match geometry.borrow().focus {
                Some(pane) => Value::Integer(i64::from(pane.get())),
                None => Value::Nil,
            })
        })?;
        pane.set("current", current)?;
    }

    {
        let geometry = geometry.clone();
        let zoomed = lua.create_function(move |_, ()| {
            Ok(match geometry.borrow().zoomed {
                Some(pane) => Value::Integer(i64::from(pane.get())),
                None => Value::Nil,
            })
        })?;
        pane.set("zoomed", zoomed)?;
    }

    {
        // The same rects `ctx:pane_rect` answers with, out of the same published geometry.
        // Geometry is a question a plugin can have without drawing — which pane is to my
        // left is asked from a key handler, where there is no `ctx`.
        let geometry = geometry.clone();
        let rect = lua.create_function(move |lua, pane: u32| {
            let pane = PaneId::from(pane);
            let found = geometry
                .borrow()
                .panes
                .iter()
                .find(|(id, _)| *id == pane)
                .map(|(_, rect)| *rect);
            match found {
                Some(found) => Ok(Value::Table(rect::to_table(lua, found)?)),
                None => Ok(Value::Nil),
            }
        })?;
        pane.set("rect", rect)?;
    }

    {
        // `ctx:pane_outer` from a key handler: what draws around a pane, or places something
        // over the panes, asks for it where there is no `ctx` too.
        let geometry = geometry.clone();
        let outer = lua.create_function(move |lua, pane: u32| {
            let pane = PaneId::from(pane);
            let geometry = geometry.borrow();
            let find = |rects: &[(PaneId, Rect)]| {
                rects
                    .iter()
                    .find(|(id, _)| *id == pane)
                    .map(|(_, rect)| *rect)
            };
            match find(&geometry.outer).or_else(|| find(&geometry.panes)) {
                Some(found) => Ok(Value::Table(rect::to_table(lua, found)?)),
                None => Ok(Value::Nil),
            }
        })?;
        pane.set("outer", outer)?;
    }

    {
        // One call rather than a `command` and a `cwd`: both facts come from the one process
        // the host looked at, and two calls could answer about two different ones.
        let info = lua.create_function(move |lua, pane: u32| {
            let pane_info = pane_info.borrow();
            let Some(info) = pane_info.get(&PaneId::from(pane)) else {
                return Ok(Value::Nil);
            };
            let table = lua.create_table()?;
            // A fact that could not be determined is simply absent, which is `nil` to read —
            // the same answer `pane.current()` gives for nothing.
            if let Some(command) = &info.command {
                table.set("command", command.as_str())?;
            }
            if let Some(cwd) = &info.cwd {
                table.set("cwd", cwd.as_str())?;
            }
            if let Some(job) = &info.job {
                let processes = lua.create_table()?;
                for (i, process) in job.processes.iter().enumerate() {
                    let entry = lua.create_table()?;
                    entry.set("pid", process.pid)?;
                    entry.set("name", process.name.as_str())?;
                    if let Some(argv) = &process.argv {
                        entry.set(
                            "argv",
                            lua.create_sequence_from(argv.iter().map(String::as_str))?,
                        )?;
                    }
                    processes.set(i + 1, entry)?;
                }
                let job_table = lua.create_table()?;
                job_table.set("group", job.group)?;
                job_table.set("processes", processes)?;
                table.set("job", job_table)?;
            }
            Ok(Value::Table(table))
        })?;
        pane.set("info", info)?;
    }

    {
        // The cheap half of reading a pane. A watcher asks this every frame for every pane
        // and reads the lines only when the answer moved, which is what keeps an idle
        // session idle. It says the pane's terminal has been fed, not that the screen it
        // shows is different.
        let terminals = terminals.clone();
        let revision = lua.create_function(move |_, pane: u32| {
            Ok(match terminals.revision(PaneId::from(pane)) {
                Some(revision) => Value::Integer(revision as i64),
                None => Value::Nil,
            })
        })?;
        pane.set("revision", revision)?;
    }

    {
        // The title, not the command: a program that sets one is saying what it is doing,
        // which is a different fact from what `pane.info` reads out of `/proc`.
        let terminals = terminals.clone();
        let title =
            lua.create_function(move |_, pane: u32| Ok(terminals.title(PaneId::from(pane))))?;
        pane.set("title", title)?;
    }

    {
        // Lines, never bytes: the host has already reconstructed the terminal, so a plugin
        // matching a pattern must not be handed escape sequences to strip. `opts.lines` is
        // how many rows from the bottom to read, and it bounds the work rather than trimming
        // an answer built in full.
        //
        // A pane nobody has answers nil rather than raising, and so does one that closed
        // between two passes of a loop: a shell exiting is not the caller's error. Which
        // panes exist is a separate question, and `pane.list` and `pane_output` answer it.
        let terminals = terminals.clone();
        let content = lua.create_function(move |lua, (pane, opts): (u32, Option<Table>)| {
            let last = match &opts {
                Some(opts) => opts.get::<Option<i64>>("lines")?,
                None => None,
            };
            let last = count("content", "lines count", last)?;
            let want = Want {
                runs: flag(opts.as_ref(), "runs")?,
                ..want(opts.as_ref())?
            };
            let Some(text) = terminals.content(PaneId::from(pane), last, want) else {
                return Ok((Value::Nil, Value::Nil, Value::Nil));
            };
            answer(lua, text)
        })?;
        pane.set("content", content)?;
    }

    {
        // The cheap half of reading the history, as `revision` is of reading the screen: two
        // numbers, with no line built to answer them. The second is what makes the first
        // safe to hold on to — a line number means a different line after the ring drops
        // one, and nothing else in the read path shows that it happened.
        let terminals = terminals.clone();
        let history = lua.create_function(move |_, pane: u32| {
            Ok(match terminals.history_extent(PaneId::from(pane)) {
                Some(history) => (
                    Value::Integer(history.len as i64),
                    Value::Integer(history.dropped as i64),
                ),
                None => (Value::Nil, Value::Nil),
            })
        })?;
        pane.set("history", history)?;
    }

    {
        // The lines above the screen. `from` is a line number in the pane's own history,
        // where 0 is the oldest line it still holds, and `lines` bounds the read the way
        // `content`'s does. Reading moves the terminal's position in its scrollback and puts
        // it back before answering, so nothing a person is looking at moves.
        let terminals = terminals.clone();
        let scrollback = lua.create_function(move |lua, (pane, opts): (u32, Option<Table>)| {
            let (from, lines) = match &opts {
                Some(opts) => (
                    opts.get::<Option<i64>>("from")?,
                    opts.get::<Option<i64>>("lines")?,
                ),
                None => (None, None),
            };
            let from = count("scrollback", "from", from)?.unwrap_or(0);
            // No count is the rest of the history, as no `lines` is the whole screen.
            let lines = count("scrollback", "lines count", lines)?.unwrap_or(u64::MAX);
            let want = want(opts.as_ref())?;
            let Some(text) = terminals.history(PaneId::from(pane), from, lines, want) else {
                return Ok((Value::Nil, Value::Nil, Value::Nil));
            };
            answer(lua, text)
        })?;
        pane.set("scrollback", scrollback)?;
    }

    {
        let frozen = lua.create_function(move |_, pane: u32| {
            Ok(match terminals.is_frozen(PaneId::from(pane)) {
                Some(frozen) => Value::Boolean(frozen),
                None => Value::Nil,
            })
        })?;
        pane.set("frozen", frozen)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let split = lua.create_function(move |lua, dir: String| {
            let dir = SplitDir::parse(&dir).ok_or_else(|| {
                mlua::Error::RuntimeError(format!(
                    "unknown split direction '{dir}', expected 'horizontal' or 'vertical'"
                ))
            })?;
            ops.push(plugin::caller(lua, &sources), Op::PaneSplit { dir });
            Ok(())
        })?;
        pane.set("split", split)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let resize = lua.create_function(move |lua, (dir, cells): (String, i64)| {
            let dir = Dir::parse(&dir).ok_or_else(|| {
                mlua::Error::RuntimeError(format!(
                    "unknown direction '{dir}', expected 'left', 'right', 'up' or 'down'"
                ))
            })?;
            // A cell count that is not a positive number of cells is a typo, not a refusal:
            // it raises, as an unparseable key spec does.
            let cells = u16::try_from(cells)
                .ok()
                .filter(|cells| *cells > 0)
                .ok_or_else(|| {
                    mlua::Error::RuntimeError(format!(
                        "pane.resize takes a positive number of cells, got {cells}"
                    ))
                })?;
            ops.push(plugin::caller(lua, &sources), Op::PaneResize { dir, cells });
            Ok(())
        })?;
        pane.set("resize", resize)?;
    }

    {
        // The focused pane, as `split` and `resize` act on it: zoom is something done to the
        // pane in front of you.
        let sources = sources.clone();
        let ops = ops.clone();
        let zoom = lua.create_function(move |lua, ()| {
            ops.push(plugin::caller(lua, &sources), Op::PaneZoom);
            Ok(())
        })?;
        pane.set("zoom", zoom)?;
    }

    {
        // Where the program put the cursor, out of the frame's own tty snapshot. `visible` is
        // whether it is showing: a hidden cursor is still somewhere, and a plugin drawing a
        // hint on it wants to know both.
        let tty = pane_tty.clone();
        let cursor = lua.create_function(move |lua, pane: u32| {
            match tty.borrow().get(&PaneId::from(pane)) {
                Some(tty) => {
                    let table = lua.create_table()?;
                    table.set("x", tty.cursor.1)?;
                    table.set("y", tty.cursor.0)?;
                    table.set("visible", tty.cursor_visible)?;
                    Ok(Value::Table(table))
                }
                None => Ok(Value::Nil),
            }
        })?;
        pane.set("cursor", cursor)?;
    }

    {
        // What the pane's program has done to the terminal. `canonical` and `echo` are the
        // tty's own flags and are absent — not false — where the host could not read them:
        // a plugin that cannot tell a password prompt from a shell must do nothing rather
        // than something.
        let tty = pane_tty.clone();
        let mode = lua.create_function(move |lua, pane: u32| {
            match tty.borrow().get(&PaneId::from(pane)) {
                Some(tty) => {
                    let table = lua.create_table()?;
                    table.set("alternate", tty.alternate)?;
                    table.set("application_cursor", tty.application_cursor)?;
                    if let Some(line) = tty.line {
                        table.set("canonical", line.canonical)?;
                        table.set("echo", line.echo)?;
                    }
                    Ok(Value::Table(table))
                }
                None => Ok(Value::Nil),
            }
        })?;
        pane.set("mode", mode)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let geometry = geometry.clone();
        // A keystroke into a pane, named the way `on_key` names one — `a`, `C-c`, `Enter`.
        //
        // The bytes are the host's to decide, because they are a fact about the pane: a pane
        // that asked for application cursor keys is written `ESC O A` where another is written
        // `ESC [ A`, and a plugin relaying somebody's arrow key cannot know which without
        // asking. `pane.send` is the other half of this — text, exactly as given — and the two
        // reach the pty by the same path, so a write hook sees a relayed key like any other.
        let key = lua.create_function(move |lua, (pane, spec): (u32, String)| {
            let pane = PaneId::from(pane);
            let held = {
                let geometry = geometry.borrow();
                geometry
                    .windows
                    .iter()
                    .any(|window| window.panes.contains(&pane))
            };
            if !held {
                return Ok((
                    false,
                    Value::String(lua.create_string(format!("no pane {pane}"))?),
                ));
            }
            // A spec nothing could mean is a programming error, the way an unparseable keymap
            // spec is, so it raises rather than being answered.
            if neotmux_proto::Key::from_spec(&spec).is_none() {
                return Err(mlua::Error::RuntimeError(format!(
                    "cannot parse key spec '{spec}'"
                )));
            }
            ops.push(plugin::caller(lua, &sources), Op::PaneKey { pane, spec });
            Ok((true, Value::Nil))
        })?;
        pane.set("key", key)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let geometry = geometry.clone();
        // Text into a pane's shell, as though it had been typed there. Bytes as written: no
        // key names, and nothing wrapped in a bracketed paste — a caller that wants either
        // writes the escapes itself, and the host does not guess at what a program expects.
        //
        // `true` is "accepted", not "arrived": like every other op this is applied after the
        // tick, so a tick that raises or overruns loses its send with the rest of its frame.
        // What can be answered now is whether the session holds that pane, and that is what
        // `false` means. A frozen pane takes it — `pane.freeze` holds the screen somebody is
        // reading, not the shell underneath it.
        let send = lua.create_function(move |lua, (pane, text): (u32, String)| {
            let pane = PaneId::from(pane);
            let held = {
                let geometry = geometry.borrow();
                // Every window's, not this frame's: the pane a plugin answers is often in a
                // window nobody is looking at.
                geometry
                    .windows
                    .iter()
                    .any(|window| window.panes.contains(&pane))
            };
            if !held {
                return Ok((
                    false,
                    Value::String(lua.create_string(format!("no pane {pane}"))?),
                ));
            }
            ops.push(plugin::caller(lua, &sources), Op::PaneSend { pane, text });
            Ok((true, Value::Nil))
        })?;
        pane.set("send", send)?;
    }

    {
        let focus = lua.create_function(move |lua, pane: u32| {
            ops.push(
                plugin::caller(lua, &sources),
                Op::PaneFocus {
                    pane: PaneId::from(pane),
                },
            );
            Ok(())
        })?;
        pane.set("focus", focus)?;
    }

    Ok(pane)
}

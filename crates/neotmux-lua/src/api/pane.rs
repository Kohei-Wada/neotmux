use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use neotmux_proto::read::{Lines, PaneRead, Trailing, Want};
use neotmux_proto::{Dir, Geometry, PaneId, SplitDir};

use crate::api::rect;
use crate::ops::{Op, OpBuffer};
use crate::paneinfo::PaneInfos;
use crate::source::{self, Sources};

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
    })
}

/// One read of a pane's text as Lua sees it: the lines, the revision they were read at, and
/// the continuation flags where they were asked for — `nil` where they were not.
fn answer(lua: &Lua, text: Lines) -> mlua::Result<(Value, Value, Value)> {
    let wrapped = match text.wrapped {
        Some(wrapped) => Value::Table(lua.create_sequence_from(wrapped)?),
        None => Value::Nil,
    };
    Ok((
        Value::Table(lua.create_sequence_from(text.lines)?),
        Value::Integer(text.revision as i64),
        wrapped,
    ))
}

/// The M2 pane API (§6), signatures frozen now. `list`, `current` and `rect` read this
/// frame's geometry; `content` and `revision` read the pane's own terminal; `split`, `focus`
/// and `resize` are ops on the buffer (§4.6), applied after the tick, so a callback never
/// sees its own request take effect synchronously.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    ops: Rc<OpBuffer>,
    geometry: Rc<RefCell<Geometry>>,
    pane_info: Rc<RefCell<PaneInfos>>,
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
            let want = want(opts.as_ref())?;
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
        // Immediate, not an op: a plugin freezes a pane in order to read it in the same
        // call, and an op applied after the tick would hand it the live text it froze to
        // get away from. Nothing about the tree changes, so there is nothing for the rest of
        // the frame to catch up with.
        let terminals = terminals.clone();
        let freeze =
            lua.create_function(move |_, pane: u32| Ok(terminals.freeze(PaneId::from(pane))))?;
        pane.set("freeze", freeze)?;
    }

    {
        let terminals = terminals.clone();
        let unfreeze =
            lua.create_function(move |_, pane: u32| Ok(terminals.unfreeze(PaneId::from(pane))))?;
        pane.set("unfreeze", unfreeze)?;
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
            ops.push(source::caller(lua, &sources), Op::PaneSplit { dir });
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
            ops.push(source::caller(lua, &sources), Op::PaneResize { dir, cells });
            Ok(())
        })?;
        pane.set("resize", resize)?;
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
            ops.push(source::caller(lua, &sources), Op::PaneSend { pane, text });
            Ok((true, Value::Nil))
        })?;
        pane.set("send", send)?;
    }

    {
        let focus = lua.create_function(move |lua, pane: u32| {
            ops.push(
                source::caller(lua, &sources),
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

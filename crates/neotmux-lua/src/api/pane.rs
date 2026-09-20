use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use neotmux_core::proto::{Dir, PaneId, SplitDir};
use neotmux_term::terminals::PaneTerminals;

use crate::api::rect;
use crate::geometry::Geometry;
use crate::ops::{Op, OpBuffer};
use crate::paneinfo::PaneInfos;
use crate::source::{self, Sources};

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
    terminals: Rc<PaneTerminals>,
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
        let content = lua.create_function(move |lua, (pane, opts): (u32, Option<Table>)| {
            let last = match opts {
                Some(opts) => opts.get::<Option<i64>>("lines")?,
                None => None,
            };
            let last = match last {
                // A negative count is a typo rather than a refusal, as a resize by a
                // negative number of cells is.
                Some(lines) => Some(usize::try_from(lines).map_err(|_| {
                    mlua::Error::RuntimeError(format!(
                        "pane.content takes a lines count of zero or more, got {lines}"
                    ))
                })?),
                None => None,
            };
            let Some((lines, revision)) = terminals.content(PaneId::from(pane), last) else {
                return Ok((Value::Nil, Value::Nil));
            };
            Ok((
                Value::Table(lua.create_sequence_from(lines)?),
                Value::Integer(revision as i64),
            ))
        })?;
        pane.set("content", content)?;
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

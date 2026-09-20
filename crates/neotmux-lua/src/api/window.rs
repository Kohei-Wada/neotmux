use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use neotmux_core::proto::WindowId;

use crate::geometry::Geometry;
use crate::ops::{Op, OpBuffer};
use crate::source::{self, Sources};
use crate::window::WindowIds;

/// `neotmux.window`, shaped like `neotmux.pane`. `list`, `panes` and `current` read this frame's
/// geometry; `create` and `focus` are ops on the buffer (§4.6), applied after the tick, so a
/// callback never sees its own request take effect synchronously — `create` answers with the
/// id the window *will* have, not with a window that is already there.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    ops: Rc<OpBuffer>,
    geometry: Rc<RefCell<Geometry>>,
    ids: Rc<WindowIds>,
) -> mlua::Result<Table> {
    let window = lua.create_table()?;

    {
        let geometry = geometry.clone();
        let list = lua.create_function(move |lua, ()| {
            let ids: Vec<u32> = geometry
                .borrow()
                .windows
                .iter()
                .map(|window| window.id.get())
                .collect();
            lua.create_sequence_from(ids)
        })?;
        window.set("list", list)?;
    }

    {
        // The panes of any window, the hidden ones included, so a plugin can find a pane
        // before it has said anything — `pane.list` stays the shown window's list. Finding
        // is all it is: nothing here acts on a pane in a window nobody is looking at.
        let geometry = geometry.clone();
        let panes = lua.create_function(move |lua, id: u32| {
            let id = WindowId::from(id);
            let found = geometry
                .borrow()
                .windows
                .iter()
                .find(|window| window.id == id)
                .map(|window| {
                    window
                        .panes
                        .iter()
                        .map(|pane| pane.get())
                        .collect::<Vec<u32>>()
                });
            match found {
                Some(found) => Ok(Value::Table(lua.create_sequence_from(found)?)),
                // Nothing, the answer `pane.current` and `pane.rect` give. A window always
                // holds at least one pane, so this is never an empty table.
                None => Ok(Value::Nil),
            }
        })?;
        window.set("panes", panes)?;
    }

    {
        let current = lua.create_function(move |_, ()| {
            Ok(match geometry.borrow().window {
                Some(id) => Value::Integer(i64::from(id.get())),
                None => Value::Nil,
            })
        })?;
        window.set("current", current)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let create = lua.create_function(move |lua, ()| {
            let id = ids.take();
            ops.push(
                source::caller(lua, &sources),
                Op::WindowCreate { window: id },
            );
            Ok(id.get())
        })?;
        window.set("create", create)?;
    }

    {
        let focus = lua.create_function(move |lua, id: u32| {
            ops.push(
                source::caller(lua, &sources),
                Op::WindowFocus {
                    window: WindowId::from(id),
                },
            );
            Ok(())
        })?;
        window.set("focus", focus)?;
    }

    Ok(window)
}

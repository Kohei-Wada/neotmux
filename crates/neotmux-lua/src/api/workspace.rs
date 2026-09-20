use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use neotmux_core::proto::WorkspaceId;

use crate::geometry::Geometry;
use crate::ops::{Op, OpBuffer};
use crate::source::{self, Sources};
use crate::workspace::WorkspaceIds;

/// `neotmux.workspace`, shaped like `neotmux.pane`. `list` and `current` read this frame's
/// geometry; `create` and `focus` are ops on the buffer (§4.6), applied after the tick, so a
/// callback never sees its own request take effect synchronously — `create` answers with the
/// id the workspace *will* have, not with a workspace that is already there.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    ops: Rc<OpBuffer>,
    geometry: Rc<RefCell<Geometry>>,
    ids: Rc<WorkspaceIds>,
) -> mlua::Result<Table> {
    let workspace = lua.create_table()?;

    {
        let geometry = geometry.clone();
        let list = lua.create_function(move |lua, ()| {
            let ids: Vec<u32> = geometry
                .borrow()
                .workspaces
                .iter()
                .map(|id| id.get())
                .collect();
            lua.create_sequence_from(ids)
        })?;
        workspace.set("list", list)?;
    }

    {
        let current = lua.create_function(move |_, ()| {
            Ok(match geometry.borrow().workspace {
                Some(id) => Value::Integer(i64::from(id.get())),
                None => Value::Nil,
            })
        })?;
        workspace.set("current", current)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let create = lua.create_function(move |lua, ()| {
            let id = ids.take();
            ops.push(
                source::caller(lua, &sources),
                Op::WorkspaceCreate { workspace: id },
            );
            Ok(id.get())
        })?;
        workspace.set("create", create)?;
    }

    {
        let focus = lua.create_function(move |lua, id: u32| {
            ops.push(
                source::caller(lua, &sources),
                Op::WorkspaceFocus {
                    workspace: WorkspaceId::from(id),
                },
            );
            Ok(())
        })?;
        workspace.set("focus", focus)?;
    }

    Ok(workspace)
}

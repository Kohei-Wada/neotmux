//! `neotmux.pane.freeze`: a hold on a pane, and the handle that lets go of it.

use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};
use neotmux_proto::PaneId;
use neotmux_proto::screen::{Hold, PaneRead};

use crate::plugin::{Groups, Held};

/// Immediate, not an op: a plugin freezes a pane in order to read it in the same call, and an
/// op applied after the tick would hand it the live text it froze to get away from. Nothing
/// about the tree changes, so there is nothing for the rest of the frame to catch up with.
pub(crate) fn install(
    lua: &Lua,
    terminals: Rc<dyn PaneRead>,
    groups: Rc<Groups>,
) -> mlua::Result<Function> {
    lua.create_function(move |lua, pane: u32| {
        Ok(match terminals.hold(PaneId::from(pane)) {
            Some(hold) => {
                groups.note(Held::Freeze(hold));
                Value::Table(handle(lua, terminals.clone(), hold)?)
            }
            None => Value::Nil,
        })
    })
}

fn handle(lua: &Lua, terminals: Rc<dyn PaneRead>, hold: Hold) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("pane", hold.pane.get())?;
    let release = lua.create_function(move |_, _self: Value| Ok(terminals.release(hold)))?;
    handle.set("release", release)?;
    Ok(handle)
}

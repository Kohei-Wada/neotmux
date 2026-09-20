use mlua::{Lua, Table};

use neotmux_proto::Rect;

/// The one shape a rect takes in Lua. `ctx:pane_rect` and `neotmux.pane.rect` read the same
/// published geometry, so they answer with the same table and cannot drift apart.
pub(crate) fn to_table(lua: &Lua, rect: Rect) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("x", rect.x)?;
    table.set("y", rect.y)?;
    table.set("w", rect.w)?;
    table.set("h", rect.h)?;
    Ok(table)
}

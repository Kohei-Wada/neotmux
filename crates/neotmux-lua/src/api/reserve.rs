use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use neotmux_core::proto::Edge;

use crate::reserve::{ReservationId, Reservations, max_reserved};
use crate::source::{self, Sources};

fn handle(lua: &Lua, reservations: Rc<Reservations>, id: ReservationId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    let remove = lua.create_function(move |_, _self: Value| {
        reservations.remove(id);
        Ok(())
    })?;
    handle.set("remove", remove)?;
    Ok(handle)
}

/// `neotmux.reserve(edge, cells)`: a band of the surface the panes no longer get.
///
/// `cells` is counted in the edge's own unit: rows at the top and the bottom, columns at the
/// left and the right. That is the only difference between the two axes, and the reason
/// there is one call rather than two.
///
/// The failures are the two §4.4 already distinguishes. An unparseable edge, a count of
/// zero, and a request no edge could ever grant are programming errors and raise, like every
/// other malformed argument in the ABI. Losing the band to someone who asked first is not:
/// the caller wrote a legal call, so it gets `nil` plus a message, exactly as `keymap.set`
/// does when it loses a key.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    reservations: Rc<Reservations>,
) -> mlua::Result<Function> {
    lua.create_function(move |lua, (edge, cells): (String, u16)| {
        let Some(parsed) = Edge::parse(&edge) else {
            return Err(mlua::Error::RuntimeError(format!(
                "unknown edge '{edge}': neotmux.reserve takes 'top', 'bottom', 'left' or \
                 'right'"
            )));
        };
        let axis = parsed.axis();
        if cells == 0 {
            return Err(mlua::Error::RuntimeError(format!(
                "neotmux.reserve takes at least one of the {axis} it measures a {parsed} band in"
            )));
        }
        let cap = max_reserved(axis);
        if cells > cap {
            return Err(mlua::Error::RuntimeError(format!(
                "neotmux.reserve takes at most {cap} {axis} on the {parsed} edge; a band is a \
                 margin on the panes, not a pane"
            )));
        }
        let owner = source::caller(lua, &sources);
        match reservations.add(owner, parsed, cells) {
            Ok(id) => Ok((
                Value::Table(handle(lua, reservations.clone(), id)?),
                Value::Nil,
            )),
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })
}

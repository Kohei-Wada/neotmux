use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use neotmux_core::proto::Edge;

use crate::reserve::{MAX_RESERVED_ROWS, ReservationId, Reservations};
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

/// `neotmux.reserve(edge, rows)`: a band of the surface the panes no longer get.
///
/// The failures are the two §4.4 already distinguishes. An unparseable edge, a row count of
/// zero, and a request no edge could ever grant are programming errors and raise, like every
/// other malformed argument in the ABI. Losing the rows to someone who asked first is not:
/// the caller wrote a legal call, so it gets `nil` plus a message, exactly as `keymap.set`
/// does when it loses a key.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    reservations: Rc<Reservations>,
) -> mlua::Result<Function> {
    lua.create_function(move |lua, (edge, rows): (String, u16)| {
        let Some(parsed) = Edge::parse(&edge) else {
            return Err(mlua::Error::RuntimeError(format!(
                "unknown edge '{edge}': neotmux.reserve takes 'top' or 'bottom'. There is no \
                 'left' or 'right' — a band is measured in rows"
            )));
        };
        if rows == 0 {
            return Err(mlua::Error::RuntimeError(
                "neotmux.reserve takes at least one row".to_string(),
            ));
        }
        if rows > MAX_RESERVED_ROWS {
            return Err(mlua::Error::RuntimeError(format!(
                "neotmux.reserve takes at most {MAX_RESERVED_ROWS} rows; a band is a line or \
                 two along an edge, not a pane"
            )));
        }
        let owner = source::caller(lua, &sources);
        match reservations.add(owner, parsed, rows) {
            Ok(id) => Ok((
                Value::Table(handle(lua, reservations.clone(), id)?),
                Value::Nil,
            )),
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })
}

use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use neotmux_proto::Edge;

use crate::draw::{ReservationId, Reservations, max_reserved};
use crate::plugin::{self, Sources};

/// What a `cells` argument has to be for either call to mean anything, whoever else is on the
/// edge: at least one of the unit the edge is measured in, and no more than the edge's bound.
/// Both are typos rather than lost races, so both raise.
fn check_cells(call: &str, edge: Edge, cells: u16) -> mlua::Result<()> {
    let axis = edge.axis();
    if cells == 0 {
        return Err(mlua::Error::RuntimeError(format!(
            "{call} takes at least one of the {axis} it measures a {edge} band in"
        )));
    }
    let cap = max_reserved(axis);
    if cells > cap {
        return Err(mlua::Error::RuntimeError(format!(
            "{call} takes at most {cap} {axis} on the {edge} edge; a band is a margin on the \
             panes, not a pane"
        )));
    }
    Ok(())
}

/// The two things an owner can do to its own band: give it back, or change how wide it is.
///
/// `handle:resize_to(cells)` sets the count, where `neotmux.pane.resize` moves a boundary by
/// one — hence the spelling, so that one verb does not mean both. The band keeps its place on
/// its edge, which is the whole reason it is not a remove and a fresh reserve.
fn handle(
    lua: &Lua,
    reservations: Rc<Reservations>,
    id: ReservationId,
    edge: Edge,
) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    let remove = {
        let reservations = reservations.clone();
        lua.create_function(move |_, _self: Value| {
            reservations.remove(id);
            Ok(())
        })?
    };
    handle.set("remove", remove)?;
    let resize_to = lua.create_function(move |lua, (_self, cells): (Value, u16)| {
        check_cells("handle:resize_to", edge, cells)?;
        match reservations.resize(id, cells) {
            Ok(()) => Ok((Value::Boolean(true), Value::Nil)),
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })?;
    handle.set("resize_to", resize_to)?;
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
/// does when it loses a key. `handle:resize_to` answers the same two ways.
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
        check_cells("neotmux.reserve", parsed, cells)?;
        let owner = plugin::caller(lua, &sources);
        match reservations.add(owner, parsed, cells) {
            Ok(id) => Ok((
                Value::Table(handle(lua, reservations.clone(), id, parsed)?),
                Value::Nil,
            )),
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })
}

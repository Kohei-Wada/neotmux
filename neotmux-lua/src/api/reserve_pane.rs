use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::draw::{MAX_PANE_BAND_COLS, PaneBands, ReservationId};
use crate::plugin::{self, Groups, Held, Sources};

/// A count no pane could ever give is a typo rather than a lost race, so it raises.
fn check_cells(call: &str, cells: u16) -> mlua::Result<()> {
    if cells == 0 {
        return Err(mlua::Error::RuntimeError(format!(
            "{call} takes at least one column"
        )));
    }
    if cells > MAX_PANE_BAND_COLS {
        return Err(mlua::Error::RuntimeError(format!(
            "{call} takes at most {MAX_PANE_BAND_COLS} columns; a pane band is a gutter, not \
             a pane"
        )));
    }
    Ok(())
}

fn handle(lua: &Lua, bands: Rc<PaneBands>, id: ReservationId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    let remove = {
        let bands = bands.clone();
        lua.create_function(move |_, _self: Value| {
            bands.remove(id);
            Ok(())
        })?
    };
    handle.set("remove", remove)?;
    let resize_to = lua.create_function(move |lua, (_self, cells): (Value, u16)| {
        check_cells("handle:resize_to", cells)?;
        match bands.resize(id, cells) {
            Ok(()) => Ok((Value::Boolean(true), Value::Nil)),
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })?;
    handle.set("resize_to", resize_to)?;
    Ok(handle)
}

/// `neotmux.reserve_pane(edge, cells)`: columns down the edge of every pane, which that pane's
/// shell no longer gets. `neotmux.reserve` one level down, answering the same two ways.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    bands: Rc<PaneBands>,
    groups: Rc<Groups>,
) -> mlua::Result<Function> {
    lua.create_function(move |lua, (edge, cells): (String, u16)| {
        if edge != "left" {
            return Err(mlua::Error::RuntimeError(format!(
                "unknown edge '{edge}': neotmux.reserve_pane takes 'left'"
            )));
        }
        check_cells("neotmux.reserve_pane", cells)?;
        let owner = plugin::caller(lua, &sources);
        match bands.add(owner, cells) {
            Ok(id) => {
                groups.note(Held::PaneBand(id));
                Ok((Value::Table(handle(lua, bands.clone(), id)?), Value::Nil))
            }
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })
}

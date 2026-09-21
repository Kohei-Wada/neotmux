use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use neotmux_proto::Rect;

use crate::float::{FloatId, Floats};
use crate::source::{self, Sources};

/// A field read as a whole Lua integer, or the message a bad one gets: `min` is the
/// smallest legal value, `0` for a corner and `1` for a dimension (spec §3, §7) —
/// `x`/`y >= 0`, `w`/`h >= 1`. A Lua float, even a whole one like `3.0`, is not a Lua
/// integer and is refused the same as a string or a missing key: the grant is meant to
/// be written as `{ x = 10, ... }`, not computed with `/`.
fn bounded(value: Value, label: &str, min: i64) -> Result<u16, String> {
    let Some(i) = value.as_i64() else {
        return Err(format!(
            "{label} must be an integer, got {}",
            value.type_name()
        ));
    };
    if i < min {
        return Err(format!("{label} must be at least {min}, got {i}"));
    }
    u16::try_from(i).map_err(|_| format!("{label} is too large: {i}"))
}

fn field(opts: &Table, name: &'static str, min: i64) -> mlua::Result<Result<u16, String>> {
    let value: Value = opts.get(name)?;
    Ok(bounded(value, &format!("neotmux.float's `{name}`"), min))
}

/// `neotmux.float`'s handle: `move_to`, `resize_to` and `remove`, the shape `reserve`'s
/// handle already carries (§3). `__id` is a private field — not part of the ABI, and
/// plugins must not rely on it — used by the host's own drawing layer (the ctx layer,
/// Task 4) to read the raw `FloatId` back out and resolve `ctx:text(win, ...)` and
/// `ctx:float_rect(win)` to the float this handle names. The handle table is not sealed,
/// so `pairs(win)` does enumerate it like any other field; privacy here is a convention,
/// not an enforced boundary.
fn handle(lua: &Lua, floats: Rc<Floats>, id: FloatId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("__id", lua.create_function(move |_, ()| Ok(id.0))?)?;

    let move_to = {
        let floats = floats.clone();
        lua.create_function(move |lua, (_self, x, y): (Value, Value, Value)| {
            match (
                bounded(x, "handle:move_to's x", 0),
                bounded(y, "handle:move_to's y", 0),
            ) {
                (Ok(x), Ok(y)) => {
                    floats.move_to(id, x, y);
                    Ok((Value::Boolean(true), Value::Nil))
                }
                (Err(message), _) | (_, Err(message)) => {
                    Ok((Value::Nil, Value::String(lua.create_string(&message)?)))
                }
            }
        })?
    };
    handle.set("move_to", move_to)?;

    let resize_to = {
        let floats = floats.clone();
        lua.create_function(move |lua, (_self, w, h): (Value, Value, Value)| {
            match (
                bounded(w, "handle:resize_to's w", 1),
                bounded(h, "handle:resize_to's h", 1),
            ) {
                (Ok(w), Ok(h)) => {
                    floats.resize(id, w, h);
                    Ok((Value::Boolean(true), Value::Nil))
                }
                (Err(message), _) | (_, Err(message)) => {
                    Ok((Value::Nil, Value::String(lua.create_string(&message)?)))
                }
            }
        })?
    };
    handle.set("resize_to", resize_to)?;

    let remove = lua.create_function(move |_, _self: Value| {
        floats.remove(id);
        Ok(())
    })?;
    handle.set("remove", remove)?;

    Ok(handle)
}

/// `neotmux.float({ x =, y =, w =, h = })`: a free rectangle in surface coordinates,
/// owed to nothing (spec §1, §2). A rect that stretches past the surface is granted and
/// clipped later, not refused here — only the four fields themselves are checked.
///
/// The failures are all §7's: a missing key, a non-integer, a negative `x`/`y`, or a
/// zero `w`/`h` are programming errors a caller can recover from, so — the `reserve`
/// convention — nothing raises and nothing is recorded; the grant answers `nil` plus a
/// message instead.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    floats: Rc<Floats>,
) -> mlua::Result<Function> {
    lua.create_function(move |lua, opts: Table| {
        let x = match field(&opts, "x", 0)? {
            Ok(x) => x,
            Err(message) => return Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        };
        let y = match field(&opts, "y", 0)? {
            Ok(y) => y,
            Err(message) => return Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        };
        let w = match field(&opts, "w", 1)? {
            Ok(w) => w,
            Err(message) => return Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        };
        let h = match field(&opts, "h", 1)? {
            Ok(h) => h,
            Err(message) => return Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        };

        let owner = source::caller(lua, &sources);
        let id = floats.add(owner, Rect::new(x, y, w, h));
        Ok((Value::Table(handle(lua, floats.clone(), id)?), Value::Nil))
    })
}

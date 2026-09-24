//! `neotmux.timer`: a plugin asks to be called again later.
//!
//! The VM has no millisecond clock of its own — `os.time` counts whole seconds — so the
//! deadline is the host's to keep, and the frame is the only thing that ever checks it.

use std::rc::Rc;
use std::time::Instant;

use mlua::{Function, Lua, Table, Value};

use crate::ambient::Ambient;
use crate::source;
use crate::source::Sources;
use crate::timers::{TimerId, Timers};

pub(crate) fn install(
    lua: &Lua,
    timers: Rc<Timers>,
    sources: Rc<Sources>,
    ambient: Rc<Ambient>,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (name, repeating) in [("every", true), ("after", false)] {
        let timers = Rc::clone(&timers);
        let sources = Rc::clone(&sources);
        let ambient = Rc::clone(&ambient);
        let arm = lua.create_function(move |lua, (ms, callback): (u64, Function)| {
            let owner = source::caller(lua, &sources);
            let key = Rc::new(lua.create_registry_value(callback)?);
            // None while an `init.lua` is running, which is where a plugin most naturally
            // arms its poll: the timer is adopted by the first session to run a frame.
            let session = ambient.current();
            match timers.arm(ms, repeating, owner, session, key, Instant::now()) {
                Ok(id) => Ok((
                    Value::Table(handle(lua, Rc::clone(&timers), id)?),
                    Value::Nil,
                )),
                Err(why) => Ok((Value::Nil, Value::String(lua.create_string(&why)?))),
            }
        })?;
        table.set(name, arm)?;
    }
    Ok(table)
}

fn handle(lua: &Lua, timers: Rc<Timers>, id: TimerId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", id.0)?;
    let stop = lua.create_function(move |_, _self: Value| Ok(timers.stop(id)))?;
    handle.set("stop", stop)?;
    Ok(handle)
}

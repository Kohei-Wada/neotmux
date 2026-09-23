use std::rc::Rc;

use mlua::{Lua, Table};

use crate::ambient::Ambient;
use crate::ops::{Op, OpBuffer};
use crate::source::{self, Sources};

/// `neotmux.session`: which sessions there are, which one the work in hand is for, and a way
/// to read one that is not.
pub(crate) fn install(
    lua: &Lua,
    ambient: Rc<Ambient>,
    sources: Rc<Sources>,
    ops: Rc<OpBuffer>,
) -> mlua::Result<Table> {
    let session = lua.create_table()?;

    {
        let ambient = ambient.clone();
        let current = lua.create_function(move |_lua, ()| {
            Ok::<Option<String>, mlua::Error>(ambient.current_name())
        })?;
        session.set("current", current)?;
    }

    {
        let ambient = ambient.clone();
        let list = lua.create_function(move |lua, ()| lua.create_sequence_from(ambient.names()))?;
        session.set("list", list)?;
    }

    {
        // The whole of reading another session: everything an ordinary read does, done against
        // a different one. `Bound`'s drop is what puts the caller back, so a raise inside the
        // call restores as surely as a return does.
        let ambient = ambient.clone();
        let with = lua.create_function(
            move |_lua, (name, body): (String, mlua::Function)| -> mlua::Result<mlua::MultiValue> {
                // Not an error: a session can end between one tick and the next, and a
                // watcher's loop is not the caller's mistake.
                let Some(id) = ambient.id_of(&name) else {
                    return Ok(mlua::MultiValue::new());
                };
                let _bound = ambient.bind(id);
                body.call::<mlua::MultiValue>(())
            },
        )?;
        session.set("with", with)?;
    }

    // The three that change the set of sessions. Ops like every other, applied after the
    // tick (§4.6), so a callback never sees its own request take effect synchronously — a
    // `create` followed by a `list` in the same tick does not name the new one.
    for (word, make) in [
        ("create", make_create as fn(String) -> Op),
        ("switch", make_switch),
        ("kill", make_kill),
    ] {
        let sources = sources.clone();
        let ops = ops.clone();
        let call = lua.create_function(move |lua, name: String| {
            ops.push(source::caller(lua, &sources), make(name));
            Ok(())
        })?;
        session.set(word, call)?;
    }

    Ok(session)
}

fn make_create(name: String) -> Op {
    Op::SessionCreate { name }
}

fn make_switch(name: String) -> Op {
    Op::SessionSwitch { name }
}

fn make_kill(name: String) -> Op {
    Op::SessionKill { name }
}

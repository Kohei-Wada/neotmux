use std::rc::Rc;

use mlua::{Lua, Table};

use crate::ambient::Ambient;

/// `neotmux.session`: which sessions there are, which one the work in hand is for, and a way
/// to read one that is not.
pub(crate) fn install(lua: &Lua, ambient: Rc<Ambient>) -> mlua::Result<Table> {
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

    Ok(session)
}

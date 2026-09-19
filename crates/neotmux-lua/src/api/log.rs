use std::rc::Rc;

use mlua::{Lua, Table};

use crate::ops::{Level, Op, OpBuffer};
use crate::plugin::Plugins;

pub fn install(lua: &Lua, plugins: Rc<Plugins>, ops: Rc<OpBuffer>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (name, level) in [
        ("info", Level::Info),
        ("warn", Level::Warn),
        ("error", Level::Error),
    ] {
        let plugins = plugins.clone();
        let ops = ops.clone();
        let f = lua.create_function(move |_, text: String| {
            ops.push(plugins.current(), Op::Log { level, text });
            Ok(())
        })?;
        table.set(name, f)?;
    }
    Ok(table)
}

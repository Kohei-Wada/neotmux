use std::rc::Rc;

use mlua::{Lua, Table};
use neotmux_proto::Level;

use crate::host::{Op, OpBuffer};
use crate::plugin::{self, Sources};

pub fn install(lua: &Lua, sources: Rc<Sources>, ops: Rc<OpBuffer>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (name, level) in [
        ("info", Level::Info),
        ("warn", Level::Warn),
        ("error", Level::Error),
    ] {
        let sources = sources.clone();
        let ops = ops.clone();
        let f = lua.create_function(move |lua, text: String| {
            ops.push(plugin::caller(lua, &sources), Op::Log { level, text });
            Ok(())
        })?;
        table.set(name, f)?;
    }
    Ok(table)
}

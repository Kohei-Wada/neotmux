use std::rc::Rc;

use mlua::{Lua, Table};

use crate::ops::{Op, OpBuffer};
use crate::source::{self, Sources};
use neotmux_proto::Level;

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
            ops.push(source::caller(lua, &sources), Op::Log { level, text });
            Ok(())
        })?;
        table.set(name, f)?;
    }
    Ok(table)
}

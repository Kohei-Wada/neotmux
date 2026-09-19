use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

#[derive(Debug, Clone)]
pub struct Opts {
    pub tick_budget_ms: u64,
    pub tick_overrun_limit: u32,
    pub prefix: String,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            tick_budget_ms: 8,
            tick_overrun_limit: 5,
            prefix: "C-b".to_string(),
        }
    }
}

pub fn install(lua: &Lua, opts: Rc<RefCell<Opts>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let meta = lua.create_table()?;
    let newindex =
        lua.create_function(move |lua, (_table, key, value): (Table, String, Value)| {
            let mut opts = opts.borrow_mut();
            match key.as_str() {
                "tick_budget_ms" => opts.tick_budget_ms = lua.unpack::<u64>(value)?,
                "tick_overrun_limit" => opts.tick_overrun_limit = lua.unpack::<u32>(value)?,
                "prefix" => opts.prefix = lua.unpack::<String>(value)?,
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "unknown neotmux.opt key '{other}'"
                    )));
                }
            }
            Ok(())
        })?;
    meta.set("__newindex", newindex)?;
    meta.set("__metatable", "neotmux.opt is sealed")?;
    table.set_metatable(Some(meta))?;
    Ok(table)
}

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
            // Unpack before borrowing: FromLua for String happens to skip __tostring today,
            // but holding the borrow across a call into mlua is safety by accident of that
            // detail, not by design (§4.6 forbids exactly this shape).
            match key.as_str() {
                "tick_budget_ms" => {
                    let value = lua.unpack::<u64>(value)?;
                    opts.borrow_mut().tick_budget_ms = value;
                }
                "tick_overrun_limit" => {
                    let value = lua.unpack::<u32>(value)?;
                    opts.borrow_mut().tick_overrun_limit = value;
                }
                "prefix" => {
                    let value = lua.unpack::<String>(value)?;
                    opts.borrow_mut().prefix = value;
                }
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

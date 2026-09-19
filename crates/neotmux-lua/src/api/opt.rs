use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

#[derive(Debug, Clone)]
pub struct Opts {
    pub tick_budget_ms: u64,
    pub tick_overrun_limit: u32,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            tick_budget_ms: 8,
            tick_overrun_limit: 5,
        }
    }
}

pub fn install(lua: &Lua, opts: Rc<RefCell<Opts>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let meta = lua.create_table()?;
    let newindex =
        lua.create_function(move |lua, (_table, key, value): (Table, String, Value)| {
            // Unpack before borrowing: a conversion can call back into mlua, and holding the
            // RefCell borrow across that call is the shape §4.6 forbids.
            match key.as_str() {
                "tick_budget_ms" => {
                    let value = lua.unpack::<u64>(value)?;
                    opts.borrow_mut().tick_budget_ms = value;
                }
                "tick_overrun_limit" => {
                    let value = lua.unpack::<u32>(value)?;
                    // A limit of zero parks on the first overrun and announces "overran its
                    // tick budget 0 times". Rejected like an unparseable key spec: it is a
                    // programming error, not a configuration. Switching the budget off is
                    // `tick_budget_ms = 0`.
                    if value == 0 {
                        return Err(mlua::Error::RuntimeError(
                            "neotmux.opt.tick_overrun_limit must be at least 1;                              neotmux.opt.tick_budget_ms = 0 turns the budget off"
                                .to_string(),
                        ));
                    }
                    opts.borrow_mut().tick_overrun_limit = value;
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

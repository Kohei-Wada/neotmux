use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

/// The widest gutter that is still a divider. The gutter is reserved at every split, and a
/// divider is one cell; the only reason to ask for more is padding on each side of it, which
/// needs three. Four is one past that and still far from eating the panes — on the smallest
/// terminal anyone runs, 80x24, a horizontal split with a gutter of four leaves each pane ten
/// rows. Beyond it the number is not a divider but a mistake (a pixel count, a colour), and is
/// refused for the same reason `tick_overrun_limit = 0` is.
pub const MAX_PANE_GUTTER: u16 = 4;

#[derive(Debug, Clone)]
pub struct Opts {
    pub tick_budget_ms: u64,
    /// What one layer's `init.lua` gets. Far larger than a tick's: a load legitimately does
    /// more work than a frame, and cutting it off at the tick budget would make plugins
    /// unloadable. It is a deadline, not a target — anything near it is a bug in the plugin,
    /// and neotmux shows nothing until the load returns.
    pub config_budget_ms: u64,
    pub tick_overrun_limit: u32,
    /// Cells left between panes. The host reserves them; what goes in them is a plugin's
    /// business and the host never learns.
    pub pane_gutter: u16,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            tick_budget_ms: 8,
            config_budget_ms: 5_000,
            tick_overrun_limit: 5,
            pane_gutter: 0,
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
                "config_budget_ms" => {
                    let value = lua.unpack::<u64>(value)?;
                    opts.borrow_mut().config_budget_ms = value;
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
                "pane_gutter" => {
                    let value = lua.unpack::<u16>(value)?;
                    if value > MAX_PANE_GUTTER {
                        return Err(mlua::Error::RuntimeError(format!(
                            "neotmux.opt.pane_gutter must be at most {MAX_PANE_GUTTER}; \
                             a gutter is a divider between panes, not a margin"
                        )));
                    }
                    opts.borrow_mut().pane_gutter = value;
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

//! `neotmux.group(name)`: nvim's augroup with `clear = true`, over every registration.

use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table};

use crate::api::Sweep;
use crate::plugin::{GroupId, Groups};

const TOP: [&str; 7] = [
    "on_tick",
    "on_event",
    "on_key",
    "on_write",
    "float",
    "reserve",
    "reserve_pane",
];
const NESTED: [(&str, &[&str]); 5] = [
    ("keymap", &["set"]),
    ("command", &["add"]),
    ("job", &["start"]),
    ("timer", &["every", "after"]),
    ("pane", &["freeze"]),
];

fn wrap(lua: &Lua, groups: &Rc<Groups>, id: GroupId, real: Function) -> mlua::Result<Function> {
    let groups = Rc::clone(groups);
    lua.create_function(move |_, args: MultiValue| {
        let _in = groups.enter(id);
        real.call::<MultiValue>(args)
    })
}

/// `neotmux` is the real table, before it is sealed: the wrappers call what is in it.
pub(crate) fn install(
    lua: &Lua,
    neotmux: &Table,
    groups: Rc<Groups>,
    sweep: Sweep,
    seal: fn(&Lua, Table, &'static str) -> mlua::Result<Table>,
) -> mlua::Result<Function> {
    let neotmux = neotmux.clone();
    lua.create_function(move |lua, name: mlua::Value| {
        let name = match name {
            mlua::Value::String(s) if !s.as_bytes().is_empty() => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::RuntimeError(
                    "neotmux.group takes a name, a non-empty string".to_string(),
                ));
            }
        };
        let (id, held) = groups.name(&name);
        sweep.empty(held);
        let g = lua.create_table()?;
        for field in TOP {
            g.set(field, wrap(lua, &groups, id, neotmux.get(field)?)?)?;
        }
        for (table, fields) in NESTED {
            let real: Table = neotmux.get(table)?;
            let sub = lua.create_table()?;
            for field in fields {
                sub.set(*field, wrap(lua, &groups, id, real.get(*field)?)?)?;
            }
            g.set(table, seal(lua, sub, "a group's table")?)?;
        }
        seal(lua, g, "a group")
    })
}

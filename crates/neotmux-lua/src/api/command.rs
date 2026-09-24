use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::api::attribute;
use crate::commands::Commands;
use crate::ops::{Op, OpBuffer};
use crate::source::Sources;
use neotmux_proto::Level;

/// `neotmux.command`: a name a plugin registers, anything can list, and anything can call by
/// name — the in-process half of the command surface spec's §4. The names live in Rust, in
/// `Commands`, rather than in a Lua table, because the CLI a later spec adds has to resolve
/// one without entering the VM.
///
/// Shipped as `command.add`, not a callable `neotmux.command(...)`: the top-level tables in
/// this ABI are sealed, and a callable table is a metatable the seal would have to make an
/// exception for. `keymap.set` is the shape this ABI already uses for "register a named
/// thing", and this follows it.
pub fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    commands: Rc<Commands>,
    ops: Rc<OpBuffer>,
) -> mlua::Result<Table> {
    let command = lua.create_table()?;

    {
        let sources = sources.clone();
        let commands = commands.clone();
        let ops = ops.clone();
        // A name already registered is refused, logged, and answered `nil, message`; the
        // first registration holds. A config layer's `init.lua` runs before the shipped one,
        // so the user's name wins over whatever a plugin would have taken it as.
        let add = lua.create_function(
            move |lua, (name, callback, opts): (String, Function, Option<Table>)| {
                let desc = match &opts {
                    Some(opts) => opts.get::<Option<String>>("desc")?,
                    None => None,
                };
                let owner = attribute(&sources, &callback);
                let key = Rc::new(lua.create_registry_value(callback)?);
                match commands.add(name.clone(), owner, desc, key) {
                    Ok(()) => Ok((Value::Boolean(true), Value::Nil)),
                    Err(holder) => {
                        let holder_name = sources.name(holder).unwrap_or_default();
                        let attempter_name = sources.name(owner).unwrap_or_default();
                        let message = format!("'{name}' is already registered by {holder_name}");
                        ops.push(
                            owner,
                            Op::Log {
                                level: Level::Warn,
                                text: format!(
                                    "command conflict on '{name}': held by {holder_name}, refused for {attempter_name}"
                                ),
                            },
                        );
                        Ok((Value::Nil, Value::String(lua.create_string(message)?)))
                    }
                }
            },
        )?;
        command.set("add", add)?;
    }

    {
        let sources = sources.clone();
        let commands = commands.clone();
        let list = lua.create_function(move |lua, ()| {
            let table = lua.create_table()?;
            for (at, info) in commands.list().into_iter().enumerate() {
                let one = lua.create_table()?;
                one.set("name", info.name)?;
                if let Some(desc) = info.desc {
                    one.set("desc", desc)?;
                }
                one.set("source", sources.name(info.owner).unwrap_or_default())?;
                table.set(at + 1, one)?;
            }
            Ok(table)
        })?;
        command.set("list", list)?;
    }

    {
        let sources = sources.clone();
        let commands = commands.clone();
        // Opens no budget region of its own: every path that can reach a Lua call at all —
        // `dispatch_key`, `dispatch_tick`, `offer_write`, `run_init`, and a command calling
        // another command through this very function — arms one before it runs any Lua, so
        // `run` is always already inside its caller's region. A dispatch path added later
        // that reaches Lua without arming one first would leave this uncovered too; the
        // budget belongs to whoever enters Lua, not to each thing Lua then calls.
        let run = lua.create_function(move |lua, (name, args): (String, Option<Table>)| {
            let Some((callback, owner)) = commands.get(&name) else {
                let message = format!("'{name}' is not a registered command");
                return Ok((
                    Value::Boolean(false),
                    Value::String(lua.create_string(message)?),
                ));
            };
            let tag = sources.name(owner).unwrap_or_default();
            let func: Function = lua.registry_value(&callback)?;
            let arg_table = match args {
                Some(table) => table,
                None => lua.create_table()?,
            };
            match func.call::<Value>(arg_table) {
                Ok(value) => match crate::commands::lines_from(value) {
                    Ok(_lines) => Ok((Value::Boolean(true), Value::Nil)),
                    Err(why) => {
                        let message = format!("{tag}: {why}");
                        Ok((
                            Value::Boolean(false),
                            Value::String(lua.create_string(message)?),
                        ))
                    }
                },
                Err(err) => {
                    let message = format!("{tag}: {err}");
                    Ok((
                        Value::Boolean(false),
                        Value::String(lua.create_string(message)?),
                    ))
                }
            }
        })?;
        command.set("run", run)?;
    }

    Ok(command)
}

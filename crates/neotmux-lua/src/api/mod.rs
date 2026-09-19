pub mod log;
pub mod opt;

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::LoadFailure;
use crate::ops::{Level, Op, OpBuffer};
use crate::plugin::Plugins;
use crate::registry::{Kind, RegistrationId, Registry};

pub use opt::Opts;

fn make_handle(lua: &Lua, registry: Rc<Registry>, id: RegistrationId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    let remove = lua.create_function(move |_, _self: Value| {
        registry.remove(id);
        Ok(())
    })?;
    handle.set("remove", remove)?;
    Ok(handle)
}

fn register(
    lua: &Lua,
    plugins: &Rc<Plugins>,
    registry: &Rc<Registry>,
    kind: Kind,
    callback: Function,
) -> mlua::Result<Table> {
    let owner = plugins.current();
    let key = Rc::new(lua.create_registry_value(callback)?);
    let id = registry.add(owner, kind, key);
    make_handle(lua, registry.clone(), id)
}

fn install_require_wrapper(
    lua: &Lua,
    plugins: Rc<Plugins>,
    registry: Rc<Registry>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
) -> mlua::Result<()> {
    let globals = lua.globals();
    let original_require: Function = globals.get("require")?;
    let wrapped = lua.create_function(move |_, modname: String| {
        match original_require.call::<Value>(modname.clone()) {
            Ok(value) => Ok(value),
            Err(err) => match modname.strip_prefix("plugin.") {
                Some(name) => {
                    let id = plugins.intern(name);
                    registry.remove_all_of(id);
                    load_failures.borrow_mut().push(LoadFailure {
                        plugin: name.to_string(),
                        message: format!("{err:#}"),
                    });
                    Ok(Value::Nil)
                }
                None => Err(err),
            },
        }
    })?;
    globals.set("require", wrapped)?;
    Ok(())
}

pub fn install(
    lua: &Lua,
    plugins: Rc<Plugins>,
    registry: Rc<Registry>,
    ops: Rc<OpBuffer>,
    opts: Rc<RefCell<Opts>>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
) -> mlua::Result<()> {
    let neotmux = lua.create_table()?;

    {
        let plugins = plugins.clone();
        let registry = registry.clone();
        let on_tick = lua.create_function(move |lua, callback: Function| {
            register(lua, &plugins, &registry, Kind::Tick, callback)
        })?;
        neotmux.set("on_tick", on_tick)?;
    }

    {
        let plugins = plugins.clone();
        let registry = registry.clone();
        let on_event = lua.create_function(move |lua, (name, callback): (String, Function)| {
            register(lua, &plugins, &registry, Kind::Event(name), callback)
        })?;
        neotmux.set("on_event", on_event)?;
    }

    {
        let keymap = lua.create_table()?;
        let plugins = plugins.clone();
        let registry = registry.clone();
        let set = lua.create_function(move |lua, (key, callback): (String, Function)| {
            register(lua, &plugins, &registry, Kind::Keymap(key), callback)
        })?;
        keymap.set("set", set)?;
        neotmux.set("keymap", keymap)?;
    }

    {
        let status = lua.create_table()?;
        let plugins = plugins.clone();
        let registry = registry.clone();
        let provider = lua.create_function(move |lua, callback: Function| {
            register(lua, &plugins, &registry, Kind::Status, callback)
        })?;
        status.set("provider", provider)?;
        neotmux.set("status", status)?;
    }

    let log = log::install(lua, plugins.clone(), ops.clone())?;
    neotmux.set("log", log)?;

    let opt = opt::install(lua, opts.clone())?;
    neotmux.set("opt", opt)?;

    {
        let plugins = plugins.clone();
        let ops = ops.clone();
        let quit = lua.create_function(move |_, ()| {
            ops.push(plugins.current(), Op::Quit);
            Ok(())
        })?;
        neotmux.set("quit", quit)?;
    }

    {
        let plugins = plugins.clone();
        let ops = ops.clone();
        let notify = lua.create_function(move |_, (level, text): (String, String)| {
            let level = match level.as_str() {
                "info" => Level::Info,
                "warn" => Level::Warn,
                "error" => Level::Error,
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "unknown notify level '{other}'"
                    )));
                }
            };
            ops.push(plugins.current(), Op::Notify { level, text });
            Ok(())
        })?;
        neotmux.set("notify", notify)?;
    }

    lua.globals().set("neotmux", neotmux)?;

    install_require_wrapper(lua, plugins, registry, load_failures)?;

    Ok(())
}

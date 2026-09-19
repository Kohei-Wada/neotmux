pub mod ctx;
pub mod event;
pub mod keymap;
pub mod log;
pub mod opt;

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::LoadFailure;
use crate::ops::{Level, Op, OpBuffer};
use crate::registry::{Kind, RegistrationId, Registry};
use crate::source::{self, SourceId, Sources};

pub use opt::Opts;

pub(crate) fn make_handle(
    lua: &Lua,
    registry: Rc<Registry>,
    id: RegistrationId,
) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    let remove = lua.create_function(move |_, _self: Value| {
        registry.remove(id);
        Ok(())
    })?;
    handle.set("remove", remove)?;
    Ok(handle)
}

/// Attribution is uniform: the owner of a registration is wherever the callback itself was
/// written, never wherever the register call happens to run. `Function::info().source` goes
/// through `lua_getinfo` in Rust, not the `debug` library, so it survives stripping `debug`
/// (§5.2), and it is lexical — a closure carries its defining chunk's source no matter when or
/// from where it is later called, which is exactly what makes `local m = require('plugin.x');
/// m.setup{}` attribute correctly without any special case for the post-`require` idiom. A
/// callback with no real chunk behind it (a native function, a `loadstring`d chunk) has no
/// source to intern and is anonymous.
pub(crate) fn attribute(sources: &Sources, callback: &Function) -> SourceId {
    sources.intern_chunk(callback.info().source.as_deref())
}

fn register(
    lua: &Lua,
    sources: &Rc<Sources>,
    registry: &Rc<Registry>,
    kind: Kind,
    callback: Function,
) -> mlua::Result<Table> {
    let owner = attribute(sources, &callback);
    let key = Rc::new(lua.create_registry_value(callback)?);
    let id = registry.add(owner, kind, key);
    make_handle(lua, registry.clone(), id)
}

fn install_require_wrapper(
    lua: &Lua,
    sources: Rc<Sources>,
    registry: Rc<Registry>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
) -> mlua::Result<()> {
    let globals = lua.globals();
    let original_require: Function = globals.get("require")?;
    let wrapped = lua.create_function(move |lua, modname: String| {
        match original_require.call::<Value>(modname.clone()) {
            Ok(value) => Ok(value),
            Err(err) => match modname.strip_prefix("plugin.") {
                Some(name) => {
                    let id = source::resolve_chunk_name(lua, &modname)?
                        .map(|chunk_name| sources.intern(&chunk_name))
                        .unwrap_or(source::ANONYMOUS);
                    registry.remove_all_of(id);
                    load_failures.borrow_mut().push(LoadFailure {
                        plugin: sources.name(id).unwrap_or_else(|| name.to_string()),
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
    sources: Rc<Sources>,
    registry: Rc<Registry>,
    ops: Rc<OpBuffer>,
    opts: Rc<RefCell<Opts>>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
) -> mlua::Result<()> {
    let neotmux = lua.create_table()?;

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let on_tick = lua.create_function(move |lua, callback: Function| {
            register(lua, &sources, &registry, Kind::Tick, callback)
        })?;
        neotmux.set("on_tick", on_tick)?;
    }

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let on_event = lua.create_function(move |lua, (name, callback): (String, Function)| {
            register(lua, &sources, &registry, Kind::Event(name), callback)
        })?;
        neotmux.set("on_event", on_event)?;
    }

    {
        let keymap = keymap::install(
            lua,
            sources.clone(),
            registry.clone(),
            ops.clone(),
            opts.clone(),
        )?;
        let keymap = seal(lua, keymap, "neotmux.keymap")?;
        neotmux.set("keymap", keymap)?;
    }

    {
        let status = lua.create_table()?;
        let sources = sources.clone();
        let registry = registry.clone();
        let provider = lua.create_function(move |lua, callback: Function| {
            register(lua, &sources, &registry, Kind::Status, callback)
        })?;
        status.set("provider", provider)?;
        let status = seal(lua, status, "neotmux.status")?;
        neotmux.set("status", status)?;
    }

    let log = log::install(lua, sources.clone(), ops.clone())?;
    let log = seal(lua, log, "neotmux.log")?;
    neotmux.set("log", log)?;

    let opt = opt::install(lua, opts.clone())?;
    neotmux.set("opt", opt)?;

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let quit = lua.create_function(move |lua, ()| {
            ops.push(source::caller(lua, &sources), Op::Quit);
            Ok(())
        })?;
        neotmux.set("quit", quit)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let notify = lua.create_function(move |lua, (level, text): (String, String)| {
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
            ops.push(source::caller(lua, &sources), Op::Notify { level, text });
            Ok(())
        })?;
        neotmux.set("notify", notify)?;
    }

    let neotmux = seal(lua, neotmux, "neotmux")?;
    lua.globals().set("neotmux", neotmux)?;

    install_require_wrapper(lua, sources, registry, load_failures)?;

    Ok(())
}

/// A plugin assigning `neotmux.on_tick = ...` or replacing `neotmux.log` wholesale would make
/// every plugin loaded afterwards register into nothing and log nowhere, silently — the
/// cross-plugin breakage the full-trust model does not cover (§7). `__newindex` alone would
/// not stop this: Lua only invokes it for a key that is not already present, and `on_tick`
/// etc. are already raw fields of `real`. So `real` is wrapped in an empty proxy table whose
/// `__index` reads through to it and whose `__newindex` rejects every write, which fires for
/// every key because the proxy itself is always empty. `__metatable` also refuses to let
/// `setmetatable` swap the guard back out.
fn seal(lua: &Lua, real: Table, name: &'static str) -> mlua::Result<Table> {
    let proxy = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", real.clone())?;
    let newindex =
        lua.create_function(move |_, (_table, key, _value): (Table, Value, Value)| {
            Err::<(), _>(mlua::Error::RuntimeError(format!(
                "{name} is read-only, cannot set {key:?}"
            )))
        })?;
    meta.set("__newindex", newindex)?;
    meta.set("__metatable", format!("{name} is sealed"))?;
    // Without __pairs the proxy is an empty table to `pairs`, so iteration silently sees
    // nothing where it previously saw the sealed API.
    let pairs_target = real;
    let pairs = lua.create_function(move |lua, _table: Table| {
        let next: Function = lua.globals().get("next")?;
        Ok((next, pairs_target.clone(), Value::Nil))
    })?;
    meta.set("__pairs", pairs)?;
    proxy.set_metatable(Some(meta))?;
    Ok(proxy)
}

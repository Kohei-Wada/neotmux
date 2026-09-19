pub mod ctx;
pub mod event;
pub mod keymap;
pub mod log;
pub mod opt;
pub mod pane;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::LoadFailure;
use crate::draw::Buffers;
use crate::geometry::Geometry;
use crate::ops::{Level, Op, OpBuffer};
use crate::overrun::Overrun;
use crate::registry::{Kind, KindTag, RegistrationId, Registry};
use crate::source::{self, SourceId, Sources};

pub use opt::Opts;

/// The state every binding is wired to, passed as one value because `install` otherwise takes
/// eight collaborators.
pub(crate) struct Shared {
    pub(crate) sources: Rc<Sources>,
    pub(crate) registry: Rc<Registry>,
    pub(crate) buffers: Rc<Buffers>,
    pub(crate) ops: Rc<OpBuffer>,
    pub(crate) opts: Rc<RefCell<Opts>>,
    pub(crate) load_failures: Rc<RefCell<Vec<LoadFailure>>>,
    pub(crate) geometry: Rc<RefCell<Geometry>>,
    pub(crate) overrun: Rc<Overrun>,
}

pub(crate) fn make_handle(
    lua: &Lua,
    registry: Rc<Registry>,
    buffers: Rc<Buffers>,
    id: RegistrationId,
) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    let remove = lua.create_function(move |_, _self: Value| {
        let owner = registry.owner(id);
        registry.remove(id);
        if let Some(owner) = owner {
            forget_if_no_longer_drawing(&registry, &buffers, owner);
        }
        Ok(())
    })?;
    handle.set("remove", remove)?;
    Ok(handle)
}

/// Drawing is only ever contributed from a tick, so a source with no tick registration left
/// can never refresh its frame. Without this its last committed frame is replayed on every
/// frame for as long as the epoch matches, which makes `handle:remove()` — and M1b's overrun
/// unsubscribe — leave the offending source's output on screen forever.
pub(crate) fn forget_if_no_longer_drawing(registry: &Registry, buffers: &Buffers, owner: SourceId) {
    let still_draws = registry
        .snapshot(KindTag::Tick)
        .into_iter()
        .any(|id| registry.owner(id) == Some(owner));
    if !still_draws {
        buffers.forget(owner);
    }
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
    buffers: &Rc<Buffers>,
    kind: Kind,
    callback: Function,
) -> mlua::Result<(SourceId, Table)> {
    let owner = attribute(sources, &callback);
    let key = Rc::new(lua.create_registry_value(callback)?);
    let id = registry.add(owner, kind, key);
    Ok((
        owner,
        make_handle(lua, registry.clone(), buffers.clone(), id)?,
    ))
}

/// The load convention, not `require`, owns isolation (§4.10). `loading` is set only while
/// `run_init` is running and `depth` counts how deep the current `require` is nested, so the
/// wrapper can tell a load-convention load — a `require` written in an init chain, with no
/// other `require` below it — from a plugin's own `require` of its dependency and from a
/// `require` reached at runtime out of a callback. Only the first is swallowed; the other two
/// raise like any other error, which is what attributes a missing dependency to the plugin
/// that wanted it.
fn install_require_wrapper(
    lua: &Lua,
    sources: Rc<Sources>,
    registry: Rc<Registry>,
    buffers: Rc<Buffers>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
) -> mlua::Result<Rc<Cell<bool>>> {
    let loading = Rc::new(Cell::new(false));
    let armed = loading.clone();
    let globals = lua.globals();
    let original_require: Function = globals.get("require")?;
    let depth = Cell::new(0u32);
    let wrapped = lua.create_function(move |lua, modname: String| {
        let nested = depth.get() > 0;
        depth.set(depth.get() + 1);
        let result = original_require.call::<Value>(modname.clone());
        depth.set(depth.get() - 1);

        let err = match result {
            Ok(value) => return Ok(value),
            Err(err) => err,
        };
        if nested || !armed.get() {
            return Err(err);
        }

        // A module that does not resolve to a file never ran, so it owns no registrations.
        // Removing "its" registrations without a resolved source would empty whichever
        // bucket the fallback names — `ANONYMOUS`, which real callbacks share.
        let failed = source::resolve_chunk_name(lua, &modname)?
            .map(|chunk_name| sources.intern(&chunk_name));
        if let Some(id) = failed {
            registry.remove_all_of(id);
            forget_if_no_longer_drawing(&registry, &buffers, id);
        }
        load_failures.borrow_mut().push(LoadFailure {
            source: failed
                .and_then(|id| sources.name(id))
                .unwrap_or_else(|| modname.clone()),
            message: format!("{err:#}"),
        });
        Ok(Value::Nil)
    })?;
    globals.set("require", wrapped)?;
    Ok(loading)
}

/// Returns the flag that arms the load convention (§4.10): the caller sets it while the
/// `init.lua` chain runs and clears it afterwards.
pub(crate) fn install(lua: &Lua, shared: Shared) -> mlua::Result<Rc<Cell<bool>>> {
    let Shared {
        sources,
        registry,
        buffers,
        ops,
        opts,
        load_failures,
        geometry,
        overrun,
    } = shared;
    let neotmux = lua.create_table()?;

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        let parked = overrun.clone();
        let on_tick = lua.create_function(move |lua, callback: Function| {
            let (owner, handle) =
                register(lua, &sources, &registry, &buffers, Kind::Tick, callback)?;
            parked.forget_parked(owner);
            Ok(handle)
        })?;
        neotmux.set("on_tick", on_tick)?;
    }

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        let on_event = lua.create_function(move |lua, (name, callback): (String, Function)| {
            Ok(register(
                lua,
                &sources,
                &registry,
                &buffers,
                Kind::Event(name),
                callback,
            )?
            .1)
        })?;
        neotmux.set("on_event", on_event)?;
    }

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        let on_key = lua.create_function(move |lua, callback: Function| {
            Ok(register(lua, &sources, &registry, &buffers, Kind::Key, callback)?.1)
        })?;
        neotmux.set("on_key", on_key)?;
    }

    {
        let keymap = keymap::install(
            lua,
            sources.clone(),
            registry.clone(),
            buffers.clone(),
            ops.clone(),
        )?;
        let keymap = seal(lua, keymap, "neotmux.keymap")?;
        neotmux.set("keymap", keymap)?;
    }

    {
        let pane = pane::install(lua, sources.clone(), ops.clone(), geometry)?;
        let pane = seal(lua, pane, "neotmux.pane")?;
        neotmux.set("pane", pane)?;
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

    {
        // Top level, not a `neotmux.plugin` namespace: the host has no plugin concept (§4.1).
        // The display name is the only handle a human has — it is what the notification
        // reported — and two layers providing the same relative path share one on purpose
        // (`source::display_name`). So a name can match more than one source, and the first
        // interned match wins: `interned()` is in interning order, which is load order — the
        // handles that come back do not say which of them it was.
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        let resubscribe = lua.create_function(move |lua, name: String| {
            let Some((id, _)) = sources
                .interned()
                .into_iter()
                .find(|(_, interned)| *interned == name)
            else {
                return Ok(Value::Nil);
            };
            // Handles, not a bool: a restored registration has a fresh id, so the handle the
            // caller held before the park is dead and this is the only way back to it. Nil
            // when nothing was restored — a restore always brings back at least one
            // registration, so an empty list is never the answer and nil cannot be mistaken
            // for one.
            let restored = overrun.restore(&registry, id);
            if restored.is_empty() {
                return Ok(Value::Nil);
            }
            let handles = lua.create_table()?;
            for registration in restored {
                handles.push(make_handle(
                    lua,
                    registry.clone(),
                    buffers.clone(),
                    registration,
                )?)?;
            }
            Ok(Value::Table(handles))
        })?;
        neotmux.set("resubscribe", resubscribe)?;
    }

    let neotmux = seal(lua, neotmux, "neotmux")?;
    lua.globals().set("neotmux", neotmux)?;

    install_require_wrapper(lua, sources, registry, buffers, load_failures)
}

/// A chunk assigning `neotmux.on_tick = ...` or replacing `neotmux.log` wholesale would make
/// everything loaded after it register into nothing and log nowhere, silently — the
/// cross-source breakage the full-trust model does not cover (§7). `__newindex` alone would
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

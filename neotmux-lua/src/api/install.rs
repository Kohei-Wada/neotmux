use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};
use neotmux_proto::screen::PaneRead;
use neotmux_proto::{Geometry, Level, PaneId};

use crate::LoadFailure;
use crate::api::Sweep;
use crate::draw::{Buffers, Floats, HlGroups, PaneBands, Reservations, TextBuffers};
use crate::host::{Op, OpBuffer, PaneInfos, PaneTtys, WindowIds};
use crate::plugin::{
    self, Commands, Groups, Held, Kind, KindTag, RegistrationId, Registry, SourceId, Sources,
};
use crate::sched::Overrun;

/// The state every binding is wired to, passed as one value because `install` otherwise takes
/// eight collaborators.
pub(crate) struct Shared {
    pub(crate) sources: Rc<Sources>,
    pub(crate) registry: Rc<Registry>,
    pub(crate) commands: Rc<Commands>,
    pub(crate) buffers: Rc<Buffers>,
    pub(crate) ops: Rc<OpBuffer>,
    pub(crate) opts: Rc<RefCell<super::Opts>>,
    pub(crate) load_failures: Rc<RefCell<Vec<LoadFailure>>>,
    pub(crate) geometry: Rc<RefCell<Geometry>>,
    pub(crate) pane_info: Rc<RefCell<PaneInfos>>,
    pub(crate) pane_tty: Rc<RefCell<PaneTtys>>,
    pub(crate) notices: Rc<RefCell<Vec<neotmux_proto::screen::Notice>>>,
    pub(crate) panes: Rc<dyn PaneRead>,
    pub(crate) clipboard: Rc<dyn crate::host::Clipboard>,
    pub(crate) jobs: Rc<crate::sched::Jobs>,
    pub(crate) timers: Rc<crate::sched::timers::Timers>,
    pub(crate) overrun: Rc<Overrun>,
    pub(crate) reservations: Rc<Reservations>,
    pub(crate) pane_bands: Rc<PaneBands>,
    pub(crate) window_ids: Rc<WindowIds>,
    pub(crate) floats: Rc<Floats>,
    pub(crate) hl_groups: Rc<HlGroups>,
    pub(crate) text_buffers: Rc<TextBuffers>,
    /// `$HOME` as the host read it, absent where the process had none.
    pub(crate) home: Option<String>,
    /// Which session this client is in, as the host knows it. Beside `home` in kind:
    /// a fact about the process rather than about a pane, and nothing in the Lua layer
    /// could set it.
    pub(crate) ambient: Rc<crate::host::Ambient>,
    pub(crate) groups: Rc<Groups>,
    pub(crate) sweep: Sweep,
    pub(crate) tick_costs: Rc<crate::sched::TickCosts>,
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
    groups: &Groups,
    kind: Kind,
    callback: Function,
) -> mlua::Result<(SourceId, Table)> {
    let owner = attribute(sources, &callback);
    let key = Rc::new(lua.create_registry_value(callback)?);
    let id = registry.add(owner, kind, key);
    groups.note(Held::Registration(id));
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
    sweep: Sweep,
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
        let failed = plugin::resolve_chunk_name(lua, &modname)?
            .map(|chunk_name| sources.intern(&chunk_name));
        if let Some(id) = failed {
            sweep.remove_all_of(id);
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
        commands,
        buffers,
        ops,
        opts,
        load_failures,
        geometry,
        pane_info,
        pane_tty,
        notices,
        panes,
        clipboard,
        jobs,
        timers,
        overrun,
        reservations,
        pane_bands,
        window_ids,
        floats,
        hl_groups,
        text_buffers,
        home,
        ambient,
        groups,
        sweep,
        tick_costs,
    } = shared;
    let neotmux = lua.create_table()?;

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        let groups = groups.clone();
        let parked = overrun.clone();
        let on_tick = lua.create_function(move |lua, callback: Function| {
            let (owner, handle) = register(
                lua,
                &sources,
                &registry,
                &buffers,
                &groups,
                Kind::Tick,
                callback,
            )?;
            parked.forget_parked(owner);
            Ok(handle)
        })?;
        neotmux.set("on_tick", on_tick)?;
    }

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        let groups = groups.clone();
        let on_event = lua.create_function(move |lua, (name, callback): (String, Function)| {
            Ok(register(
                lua,
                &sources,
                &registry,
                &buffers,
                &groups,
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
        let groups = groups.clone();
        let on_key = lua.create_function(move |lua, callback: Function| {
            Ok(register(
                lua,
                &sources,
                &registry,
                &buffers,
                &groups,
                Kind::Key,
                callback,
            )?
            .1)
        })?;
        neotmux.set("on_key", on_key)?;
    }

    {
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        let groups = groups.clone();
        // Every write of text into a pane, before it is written. A handler answers nothing to
        // pass it and `false, why` to refuse it; `on_key` is the other half of this, for the
        // keys a person types, which are not writes.
        let on_write = lua.create_function(move |lua, callback: Function| {
            Ok(register(
                lua,
                &sources,
                &registry,
                &buffers,
                &groups,
                Kind::Write,
                callback,
            )?
            .1)
        })?;
        neotmux.set("on_write", on_write)?;
    }

    {
        let keymap = super::keymap::install(
            lua,
            sources.clone(),
            registry.clone(),
            buffers.clone(),
            ops.clone(),
            groups.clone(),
        )?;
        let keymap = seal(lua, keymap, "neotmux.keymap")?;
        neotmux.set("keymap", keymap)?;
    }

    {
        let command = super::command::install(
            lua,
            sources.clone(),
            commands.clone(),
            ops.clone(),
            groups.clone(),
        )?;
        let command = seal(lua, command, "neotmux.command")?;
        neotmux.set("command", command)?;
    }

    {
        let pane = super::pane::install(
            lua,
            sources.clone(),
            ops.clone(),
            geometry.clone(),
            pane_info,
            pane_tty,
            panes.clone(),
        )?;
        pane.set(
            "freeze",
            super::freeze::install(lua, panes.clone(), groups.clone())?,
        )?;
        // How far back `ctx:view` has the pane drawn as the frame stands: whatever a source
        // that ticked earlier asked for this frame, else what was asked last frame, and no
        // further than the history goes, which is as far as the host will wind it. Anything
        // drawn beside a pane's lines has to wind back with it, or it sits beside the wrong one.
        let views = buffers.clone();
        let extents = panes.clone();
        pane.set(
            "view",
            lua.create_function(move |_, id: u32| {
                let pane = PaneId::from(id);
                let asked = views.applicable_views().get(&pane).copied().unwrap_or(0);
                let held = extents
                    .history_extent(pane)
                    .map_or(0, |extent| extent.len as usize);
                Ok(asked.min(held))
            })?,
        )?;
        let pane = seal(lua, pane, "neotmux.pane")?;
        neotmux.set("pane", pane)?;
    }

    {
        let window =
            super::window::install(lua, sources.clone(), ops.clone(), geometry, window_ids)?;
        let window = seal(lua, window, "neotmux.window")?;
        neotmux.set("window", window)?;
    }

    {
        // `neotmux.notify` raises one; this is the surface they land on, for whoever draws it.
        let notifications = lua.create_table()?;
        {
            let notices = notices.clone();
            let list = lua.create_function(move |lua, ()| {
                let table = lua.create_table()?;
                for (at, notice) in notices.borrow().iter().enumerate() {
                    let one = lua.create_table()?;
                    one.set(
                        "level",
                        match notice.level {
                            neotmux_proto::Level::Info => "info",
                            neotmux_proto::Level::Warn => "warn",
                            neotmux_proto::Level::Error => "error",
                        },
                    )?;
                    one.set("tag", notice.tag.clone())?;
                    one.set("text", notice.text.clone())?;
                    table.set(at + 1, one)?;
                }
                Ok(table)
            })?;
            notifications.set("list", list)?;
        }
        {
            let sources = sources.clone();
            let ops = ops.clone();
            let clear = lua.create_function(move |lua, ()| {
                ops.push(plugin::caller(lua, &sources), Op::NoticesClear);
                Ok(())
            })?;
            notifications.set("clear", clear)?;
        }
        let notifications = seal(lua, notifications, "neotmux.notifications")?;
        neotmux.set("notifications", notifications)?;
    }

    {
        let session = super::session::install(lua, ambient.clone(), sources.clone(), ops.clone())?;
        let session = seal(lua, session, "neotmux.session")?;
        neotmux.set("session", session)?;
    }

    {
        let clipboard = super::clipboard::install(lua, clipboard, ambient.clone())?;
        let clipboard = seal(lua, clipboard, "neotmux.clipboard")?;
        neotmux.set("clipboard", clipboard)?;
    }

    let log = super::log::install(lua, sources.clone(), ops.clone())?;
    let log = seal(lua, log, "neotmux.log")?;
    neotmux.set("log", log)?;

    let hl = super::hl::install(lua, hl_groups.clone())?;
    let hl = seal(lua, hl, "neotmux.hl")?;
    neotmux.set("hl", hl)?;

    let buf = super::buf::install(lua, sources.clone(), text_buffers.clone(), groups.clone())?;
    let buf = seal(lua, buf, "neotmux.buf")?;
    neotmux.set("buf", buf)?;

    let opt = super::opt::install(lua, opts.clone())?;
    neotmux.set("opt", opt)?;

    let regex = super::regex::install(lua)?;
    let regex = seal(lua, regex, "neotmux.regex")?;
    neotmux.set("regex", regex)?;

    // A fact about the process, beside `pane.info`'s facts about a pane, rather than an
    // `opt`: nothing sets it, and the VM has no `os.getenv` to read it any other way (§5.2).
    // Absent, not "", where there was no `HOME` — the same answer the rest of the ABI gives.
    if let Some(home) = home {
        neotmux.set("home", home)?;
    }

    {
        // `nil` during a load: a load happens before any session does, and every other
        // per-session fact is empty there for the same reason.
        let ambient = ambient.clone();
        let session_name = lua.create_function(move |_lua, (): ()| {
            Ok::<Option<String>, mlua::Error>(ambient.current_name())
        })?;
        neotmux.set("session_name", session_name)?;
    }

    {
        // The renderer's own rule, out of `neotmux-proto`, so a plugin's columns and the
        // renderer's columns cannot disagree: `pane.content` collapses a wide character to
        // one entry at the column it starts in, and everything that takes a column — a
        // tint, a rect, a draw — counts cells. `#line` counts neither.
        let width = lua.create_function(move |_, text: String| {
            Ok(neotmux_proto::screen::width::of_str(&text) as i64)
        })?;
        neotmux.set("width", width)?;
    }

    {
        // What each source's ticks have cost since the VM was built, most first: the numbers
        // to decide by whether drawing every frame is worth changing, rather than a guess.
        let sources = sources.clone();
        let costs = lua.create_function(move |lua, (): ()| {
            let mut rows = tick_costs.snapshot();
            rows.sort_by_key(|(_, cost)| std::cmp::Reverse(cost.total));
            let out = lua.create_table()?;
            for (i, (source, cost)) in rows.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set(
                    "source",
                    sources.name(source).unwrap_or_else(|| source.to_string()),
                )?;
                row.set("ticks", cost.ticks)?;
                row.set("total_ms", cost.total.as_secs_f64() * 1000.0)?;
                row.set("longest_ms", cost.longest.as_secs_f64() * 1000.0)?;
                row.set(
                    "recent_ms",
                    lua.create_sequence_from(cost.recent.iter().map(|d| d.as_secs_f64() * 1000.0))?,
                )?;
                out.set(i + 1, row)?;
            }
            Ok(out)
        })?;
        neotmux.set("costs", costs)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let quit = lua.create_function(move |lua, ()| {
            ops.push(plugin::caller(lua, &sources), Op::Quit);
            Ok(())
        })?;
        neotmux.set("quit", quit)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let reload = lua.create_function(move |lua, ()| {
            ops.push(plugin::caller(lua, &sources), Op::Reload);
            Ok(())
        })?;
        neotmux.set("reload", reload)?;
    }

    {
        let sources = sources.clone();
        let ops = ops.clone();
        let detach = lua.create_function(move |lua, ()| {
            ops.push(plugin::caller(lua, &sources), Op::Detach);
            Ok(())
        })?;
        neotmux.set("detach", detach)?;
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
            ops.push(plugin::caller(lua, &sources), Op::Notify { level, text });
            Ok(())
        })?;
        neotmux.set("notify", notify)?;
    }

    {
        // Zeroed when the subsystem is built rather than at the epoch: a `Duration` since an
        // arbitrary start is what a monotonic clock can honestly answer, and milliseconds are
        // what every timing a plugin does is written in.
        let start = std::time::Instant::now();
        let now = lua.create_function(move |_, ()| Ok(start.elapsed().as_millis() as u64))?;
        neotmux.set("now", now)?;
    }

    {
        // Top level, not a `neotmux.plugin` namespace: the host has no plugin concept (§4.1).
        // The display name is the only handle a human has — it is what the notification
        // reported — and two layers providing the same relative path share one on purpose
        // (`plugin::display_name`). So a name can match more than one source, and the first
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

    {
        let reserve =
            super::reserve::install(lua, sources.clone(), reservations.clone(), groups.clone())?;
        neotmux.set("reserve", reserve)?;
    }

    {
        let reserve_pane =
            super::reserve_pane::install(lua, sources.clone(), pane_bands.clone(), groups.clone())?;
        neotmux.set("reserve_pane", reserve_pane)?;
    }

    {
        let float = super::float::install(lua, sources.clone(), floats.clone(), groups.clone())?;
        neotmux.set("float", float)?;
    }

    {
        let job = super::job::install(
            lua,
            jobs.clone(),
            sources.clone(),
            ambient.clone(),
            opts.clone(),
            groups.clone(),
        )?;
        let job = seal(lua, job, "neotmux.job")?;
        neotmux.set("job", job)?;
    }

    {
        let timer = super::timer::install(
            lua,
            timers.clone(),
            sources.clone(),
            ambient.clone(),
            groups.clone(),
        )?;
        let timer = seal(lua, timer, "neotmux.timer")?;
        neotmux.set("timer", timer)?;
    }

    {
        let group = super::group::install(lua, &neotmux, groups.clone(), sweep.clone(), seal)?;
        neotmux.set("group", group)?;
    }

    let neotmux = seal(lua, neotmux, "neotmux")?;
    lua.globals().set("neotmux", neotmux)?;

    install_require_wrapper(lua, sources, sweep, load_failures)
}

/// A chunk assigning `neotmux.on_tick = ...` or replacing `neotmux.log` wholesale would make
/// everything loaded after it register into nothing and log nowhere, silently — the
/// cross-source breakage the full-trust model does not cover (§7). `__newindex` alone would
/// not stop this: Lua only invokes it for a key that is not already present, and `on_tick`
/// etc. are already raw fields of `real`. So `real` is wrapped in an empty proxy table whose
/// `__index` reads through to it and whose `__newindex` rejects every write, which fires for
/// every key because the proxy itself is always empty. `__metatable` also refuses to let
/// `setmetatable` swap the guard back out.
pub(crate) fn seal(lua: &Lua, real: Table, name: &'static str) -> mlua::Result<Table> {
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

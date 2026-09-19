use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::{Level, Op};
use neotmux_lua::registry::KindTag;
use neotmux_lua::runtime::Layers;

fn layer(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    dir
}

#[test]
fn a_plugin_can_register_and_is_attributed() {
    let dir = layer(&[
        ("init.lua", "require('plugin.clock')"),
        (
            "lua/plugin/clock.lua",
            "neotmux.on_tick(function() end) neotmux.status.provider(function() end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert!(failures.is_empty(), "{failures:?}");

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1);
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.sources().name(owner).as_deref(), Some("plugin/clock"));
    assert_eq!(sub.registry().snapshot(KindTag::Status).len(), 1);
}

#[test]
fn a_handle_removes_its_registration_and_is_idempotent() {
    let dir = layer(&[
        ("init.lua", "require('plugin.fickle')"),
        (
            "lua/plugin/fickle.lua",
            "local h = neotmux.on_tick(function() end) h:remove() h:remove() return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert!(sub.registry().snapshot(KindTag::Tick).is_empty());
}

#[test]
fn a_plugin_that_raises_is_inert_and_withdraws_its_registrations() {
    let dir = layer(&[
        ("init.lua", "require('plugin.ok') require('plugin.bad')"),
        (
            "lua/plugin/ok.lua",
            "neotmux.on_tick(function() end) return {}",
        ),
        (
            "lua/plugin/bad.lua",
            "neotmux.on_tick(function() end) error('nope')",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("nope"));

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1, "the failing plugin's registration survived");
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.sources().name(owner).as_deref(), Some("plugin/ok"));
}

#[test]
fn a_broken_init_still_leaves_a_working_subsystem() {
    let dir = layer(&[("init.lua", "this is not lua")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(sub.registry().is_empty());
}

#[test]
fn no_layers_at_all_is_a_normal_startup() {
    let sub = LuaSubsystem::build(Layers::from_paths(vec![])).unwrap();
    assert!(sub.run_init().is_empty());
    assert!(sub.registry().is_empty());
}

#[test]
fn ops_reach_the_host_with_their_owner() {
    let dir = layer(&[
        ("init.lua", "require('plugin.loud')"),
        (
            "lua/plugin/loud.lua",
            "neotmux.log.warn('hi') neotmux.quit() return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 2);
    assert_eq!(sub.sources().name(ops[0].0).as_deref(), Some("plugin/loud"));
    assert_eq!(
        ops[0].1,
        Op::Log {
            level: Level::Warn,
            text: "hi".to_string()
        }
    );
    assert_eq!(ops[1].1, Op::Quit);
}

#[test]
fn opt_defaults_match_the_spec_before_any_init_lua_runs() {
    let sub = LuaSubsystem::build(Layers::from_paths(vec![])).unwrap();
    let opt = sub.opt();
    assert_eq!(opt.tick_budget_ms, 8);
    assert_eq!(opt.tick_overrun_limit, 5);
}

#[test]
fn opt_sets_host_settings_and_rejects_unknown_keys() {
    let dir = layer(&[("init.lua", "neotmux.opt.tick_budget_ms = 12")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.opt().tick_budget_ms, 12);

    let dir = layer(&[("init.lua", "neotmux.opt.no_such_option = 1")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("no_such_option"));
}

#[test]
fn a_setup_call_after_require_is_attributed_to_the_plugin_not_init() {
    let dir = layer(&[
        ("init.lua", "local m = require('plugin.clock') m.setup()"),
        (
            "lua/plugin/clock.lua",
            "return { setup = function() neotmux.on_tick(function() end) end }",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1);
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.sources().name(owner).as_deref(), Some("plugin/clock"));
}

#[test]
fn a_plugin_required_twice_still_attributes_its_cached_table_correctly() {
    let dir = layer(&[
        (
            "init.lua",
            "require('plugin.clock') local m = require('plugin.clock') m.setup()",
        ),
        (
            "lua/plugin/clock.lua",
            "return { setup = function() neotmux.on_tick(function() end) end }",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1);
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.sources().name(owner).as_deref(), Some("plugin/clock"));
}

#[test]
fn a_bare_init_lua_registration_is_still_attributed_to_init() {
    let dir = layer(&[("init.lua", "neotmux.on_tick(function() end)")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1);
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.sources().name(owner).as_deref(), Some("init"));
}

#[test]
fn a_library_helper_registering_on_a_plugins_behalf_is_attributed_to_the_callbacks_chunk() {
    let dir = layer(&[
        ("init.lua", "require('plugin.user')"),
        (
            "lua/neotmux/helper.lua",
            "return { register = function(cb) neotmux.on_tick(cb) end }",
        ),
        (
            "lua/plugin/user.lua",
            "local helper = require('neotmux.helper') helper.register(function() end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1);
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.sources().name(owner).as_deref(), Some("plugin/user"));
}

#[test]
fn a_registration_made_during_a_dispatch_is_owned_by_the_chunk_it_was_written_in() {
    let dir = layer(&[
        ("init.lua", "require('plugin.spawner')"),
        (
            "lua/plugin/spawner.lua",
            "neotmux.on_tick(function() neotmux.on_tick(function() end) end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    let before = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(before.len(), 1);

    sub.dispatch_tick();

    let after = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(after.len(), 2);
    for id in after {
        let owner = sub.registry().owner(id).unwrap();
        assert_eq!(sub.sources().name(owner).as_deref(), Some("plugin/spawner"));
    }
}

#[test]
fn a_callback_written_in_a_library_and_registered_during_a_dispatch_is_the_librarys() {
    let dir = layer(&[
        ("init.lua", "require('plugin.spawner')"),
        (
            "lua/neotmux/factory.lua",
            "return { make = function() return function() end end }",
        ),
        (
            "lua/plugin/spawner.lua",
            "local factory = require('neotmux.factory') neotmux.on_tick(function() neotmux.on_tick(factory.make()) end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    sub.dispatch_tick();

    let after = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(after.len(), 2);
    let owners: Vec<String> = after
        .iter()
        .map(|id| {
            sub.sources()
                .name(sub.registry().owner(*id).unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(owners, vec!["plugin/spawner", "neotmux/factory"]);
}

#[test]
fn requiring_plugin_foo_init_and_plugin_foo_share_one_id() {
    let dir = layer(&[
        (
            "init.lua",
            "local m = require('plugin.foo') m.setup() require('plugin.foo.init')",
        ),
        (
            "lua/plugin/foo/init.lua",
            "neotmux.on_event('chunk', function() end) return { setup = function() neotmux.on_event('setup', function() end) end }",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());

    let events = sub.registry().snapshot(KindTag::Event);
    assert_eq!(
        events.len(),
        3,
        "chunk body ran twice plus one setup() call"
    );
    let owners: Vec<_> = events
        .iter()
        .map(|id| sub.registry().owner(*id).unwrap())
        .collect();
    assert!(owners.windows(2).all(|w| w[0] == w[1]), "{owners:?}");
    assert_eq!(
        sub.sources().name(owners[0]).as_deref(),
        Some("plugin/foo/init")
    );

    let interned = sub.sources().interned();
    assert!(
        interned.iter().all(|(_, name)| name != "plugin/foo"),
        "a phantom 'plugin/foo' source was interned distinct from plugin/foo/init: {interned:?}"
    );
}

#[test]
fn a_plugin_cannot_overwrite_a_neotmux_field() {
    let dir = layer(&[("init.lua", "neotmux.on_tick = function() end")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("read-only"));
}

#[test]
fn a_plugin_cannot_swap_the_neotmux_metatable() {
    let dir = layer(&[("init.lua", "setmetatable(neotmux, {})")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
}

#[test]
fn a_plugin_cannot_swap_the_opt_metatable() {
    let dir = layer(&[("init.lua", "setmetatable(neotmux.opt, {})")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
}

#[test]
fn neotmux_opt_assignment_still_works_once_sealed() {
    let dir = layer(&[("init.lua", "neotmux.opt.tick_overrun_limit = 9")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.opt().tick_overrun_limit, 9);
}

#[test]
fn setting_every_opt_key_in_one_statement_does_not_conflict() {
    // opt's __newindex used to hold opts.borrow_mut() across lua.unpack(value). It now
    // unpacks into a local before borrowing, so back-to-back writes in the same chunk never
    // overlap a borrow with a call back into mlua.
    let dir = layer(&[(
        "init.lua",
        "neotmux.opt.tick_budget_ms = 12 neotmux.opt.tick_overrun_limit = 3",
    )]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.opt().tick_budget_ms, 12);
    assert_eq!(sub.opt().tick_overrun_limit, 3);
}

#[test]
fn a_plugin_cannot_monkey_patch_neotmux_log() {
    let dir = layer(&[("init.lua", "neotmux.log.info = function() end")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("read-only"));
}

#[test]
fn a_plugin_cannot_monkey_patch_neotmux_keymap() {
    let dir = layer(&[("init.lua", "neotmux.keymap.set = function() end")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("read-only"));
}

#[test]
fn a_plugin_cannot_monkey_patch_neotmux_status() {
    let dir = layer(&[("init.lua", "neotmux.status.provider = function() end")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("read-only"));
}

#[test]
fn a_plugin_cannot_monkey_patch_neotmux_pane() {
    let dir = layer(&[("init.lua", "neotmux.pane.split = function() end")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("read-only"));
}

#[test]
fn sealed_subtable_functions_still_work_through_the_proxy() {
    let dir = layer(&[(
        "init.lua",
        "neotmux.log.info('from log') neotmux.keymap.set('x', function() end) neotmux.status.provider(function() end)",
    )]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    let ops = sub.drain_ops();
    assert!(ops.iter().any(|(_, op)| matches!(
        op,
        Op::Log { text, .. } if text == "from log"
    )));
    assert_eq!(sub.registry().snapshot(KindTag::Keymap).len(), 1);
    assert_eq!(sub.registry().snapshot(KindTag::Status).len(), 1);
}

#[test]
fn pairs_over_the_sealed_top_level_table_sees_the_real_fields() {
    let dir = layer(&[(
        "init.lua",
        r#"
        local names = {}
        for name in pairs(neotmux) do names[#names + 1] = name end
        table.sort(names)
        neotmux.notify("info", table.concat(names, " "))
        "#,
    )]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    let ops = sub.drain_ops();
    let (_, op) = ops
        .into_iter()
        .find(|(_, op)| matches!(op, Op::Notify { .. }))
        .expect("no notify op");
    let Op::Notify { text, .. } = op else {
        unreachable!()
    };
    assert_eq!(
        text, "keymap log notify on_event on_tick opt pane quit status",
        "the top-level ABI surface changed"
    );
}

#[test]
fn pairs_over_a_sealed_subtable_sees_its_real_fields() {
    let dir = layer(&[(
        "init.lua",
        r#"
        local count = 0
        for _ in pairs(neotmux.log) do count = count + 1 end
        neotmux.notify("info", tostring(count))
        "#,
    )]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    let ops = sub.drain_ops();
    let (_, op) = ops
        .into_iter()
        .find(|(_, op)| matches!(op, Op::Notify { .. }))
        .expect("no notify op");
    let Op::Notify { text, .. } = op else {
        unreachable!()
    };
    assert_eq!(text, "3");
}

/// The owner of an op is the chunk it was written in, so this and the two tests below all
/// read the tag the host would put on the log line.
fn log_owner(sub: &LuaSubsystem) -> String {
    let ops = sub.drain_ops();
    let (id, _) = ops
        .iter()
        .find(|(_, op)| matches!(op, Op::Log { .. }))
        .expect("no log op");
    sub.sources().name(*id).expect("the owner has no name")
}

#[test]
fn a_log_call_from_a_plugin_function_invoked_by_init_is_attributed_to_the_plugin() {
    let dir = layer(&[
        (
            "init.lua",
            "require('plugin.clock').setup { format = 'HH:MM' }",
        ),
        (
            "lua/plugin/clock.lua",
            "local M = {} function M.setup(o) neotmux.status.provider(function() end) neotmux.log.info('clock configured') end return M",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());

    let status = sub.registry().snapshot(KindTag::Status);
    assert_eq!(status.len(), 1);
    let registered_to = sub
        .sources()
        .name(sub.registry().owner(status[0]).unwrap())
        .unwrap();
    assert_eq!(registered_to, "plugin/clock");
    assert_eq!(
        log_owner(&sub),
        "plugin/clock",
        "the log line and the registration in the same function must have the same owner"
    );
}

#[test]
fn a_log_call_from_init_lua_top_level_is_attributed_to_init() {
    let dir = layer(&[("init.lua", "neotmux.log.info('straight from init')")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert_eq!(log_owner(&sub), "init");
}

#[test]
fn a_log_call_reached_through_a_library_is_attributed_to_the_library() {
    let dir = layer(&[
        ("init.lua", "require('plugin.user')"),
        (
            "lua/neotmux/helper.lua",
            "return { say = function(text) neotmux.log.info(text) end }",
        ),
        (
            "lua/plugin/user.lua",
            "require('neotmux.helper').say('via the library') return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert_eq!(log_owner(&sub), "neotmux/helper");
}

#[test]
fn a_notify_from_a_plugin_tick_callback_is_attributed_to_the_plugin() {
    let dir = layer(&[
        ("init.lua", "require('plugin.ticker')"),
        (
            "lua/plugin/ticker.lua",
            "neotmux.on_tick(function() neotmux.notify('warn', 'tick') end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    sub.dispatch_tick();

    let ops = sub.drain_ops();
    let (id, _) = ops
        .iter()
        .find(|(_, op)| matches!(op, Op::Notify { .. }))
        .expect("no notify op");
    assert_eq!(sub.sources().name(*id).as_deref(), Some("plugin/ticker"));
}

#[test]
fn an_unresolvable_require_leaves_anonymous_registrations_alone() {
    // The callback is written inside a `load(...)` chunk, so it has no `@path` source and is
    // owned by ANONYMOUS — the bucket an unresolvable module would otherwise be filed under.
    let dir = layer(&[(
        "init.lua",
        "neotmux.on_tick(load('return function() end')()) require('plugin.does_not_exist')",
    )]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].plugin, "plugin.does_not_exist", "{failures:?}");
    assert_eq!(
        sub.registry().snapshot(KindTag::Tick).len(),
        1,
        "an unresolvable require wiped registrations it never owned"
    );
}

#[test]
fn a_plugins_own_failing_require_is_attributed_to_the_plugin() {
    let dir = layer(&[
        ("init.lua", "require('plugin.ok') require('plugin.needy')"),
        (
            "lua/plugin/ok.lua",
            "neotmux.on_tick(function() end) return {}",
        ),
        (
            "lua/plugin/needy.lua",
            "neotmux.on_tick(function() end) require('plugin.absent_dep') neotmux.on_tick(function() end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(
        failures[0].plugin, "plugin/needy",
        "the outer load is the failure, not the dependency it asked for"
    );
    assert!(failures[0].message.contains("absent_dep"), "{failures:?}");

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(
        ticks.len(),
        1,
        "the failing plugin's registrations survived"
    );
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.sources().name(owner).as_deref(), Some("plugin/ok"));
}

#[test]
fn a_require_outside_init_raises_instead_of_returning_nil() {
    let dir = layer(&[
        ("init.lua", "require('plugin.later')"),
        (
            "lua/plugin/later.lua",
            "neotmux.on_tick(function() local ok = pcall(require, 'plugin.absent') neotmux.log.info('ok=' .. tostring(ok)) end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    sub.dispatch_tick();

    let text = format!("{:?}", sub.drain_ops());
    assert!(
        text.contains("ok=false"),
        "a require at runtime should raise, not be swallowed by the load convention: {text}"
    );
}

#[test]
fn a_quit_called_through_pcall_is_attributed_to_the_calling_chunk() {
    let dir = layer(&[("init.lua", "pcall(neotmux.quit)")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());

    let ops = sub.drain_ops();
    let (id, _) = ops
        .iter()
        .find(|(_, op)| matches!(op, Op::Quit))
        .expect("no quit op");
    assert_eq!(sub.sources().name(*id).as_deref(), Some("init"));
}

#[test]
fn a_log_call_from_inside_a_resumed_coroutine_is_attributed_to_its_chunk() {
    let dir = layer(&[
        ("init.lua", "require('plugin.threaded')"),
        (
            "lua/plugin/threaded.lua",
            "local co = coroutine.create(function() neotmux.log.info('from the coroutine') end) neotmux.on_tick(function() coroutine.resume(co) end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    sub.dispatch_tick();

    let ops = sub.drain_ops();
    let (id, _) = ops
        .iter()
        .find(|(_, op)| matches!(op, Op::Log { text, .. } if text == "from the coroutine"))
        .expect("the coroutine's log line never arrived");
    assert_eq!(sub.sources().name(*id).as_deref(), Some("plugin/threaded"));
}

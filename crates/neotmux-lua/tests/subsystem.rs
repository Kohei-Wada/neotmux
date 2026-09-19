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
    assert_eq!(sub.plugins().name(owner).as_deref(), Some("clock"));
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
    assert_eq!(sub.plugins().name(owner).as_deref(), Some("ok"));
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
    assert_eq!(sub.plugins().name(ops[0].0).as_deref(), Some("loud"));
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
fn a_registration_made_during_a_dispatch_inherits_the_dispatched_plugin() {
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
        assert_eq!(sub.plugins().name(owner).as_deref(), Some("spawner"));
    }
}

//! A reload: `unload`, forget the layers' modules, run every `init.lua` again, sweep the
//! groups nobody named.

use std::fs;
use std::path::Path;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::Op;
use neotmux_lua::plugin::{KindTag, Layers};
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};

mod support;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        write(dir.path(), rel, body);
    }
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(None)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn notified(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn frame(sub: &LuaSubsystem) {
    let size = Size { rows: 24, cols: 80 };
    let pane = PaneId::from(1);
    sub.begin_frame(
        support::one(),
        Epoch::new(size, vec![pane], Some(pane)),
        Geometry {
            area: Rect::new(0, 0, size.cols, size.rows),
            panes: vec![(pane, Rect::new(0, 0, 80, 24))],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());
}

const CLOCK: &str = "local g = neotmux.group('plugin.clock')
                     g.on_tick(function() end)
                     g.on_event('unload', function() neotmux.log.info('clock unloading') end)";

#[test]
fn a_reload_runs_the_new_code_and_leaves_none_of_the_old() {
    let (dir, sub) = built(&[
        ("lua/plugin/clock.lua", CLOCK),
        ("init.lua", "require('plugin.clock')"),
    ]);
    write(
        dir.path(),
        "lua/plugin/clock.lua",
        "local g = neotmux.group('plugin.clock')
         g.on_tick(function() end)
         g.on_tick(function() end)",
    );
    assert!(sub.reload(None).is_empty());
    assert_eq!(sub.registry().snapshot(KindTag::Tick).len(), 2);
}

#[test]
fn unload_runs_once_before_the_reload() {
    let (_dir, sub) = built(&[
        ("lua/plugin/clock.lua", CLOCK),
        ("init.lua", "require('plugin.clock')"),
    ]);
    logged(&sub);
    sub.reload(None);
    let said = logged(&sub);
    assert_eq!(
        said.iter().filter(|l| *l == "clock unloading").count(),
        1,
        "{said:?}"
    );
}

#[test]
fn a_plugin_whose_require_was_deleted_is_gone_after_a_reload() {
    let (dir, sub) = built(&[
        ("lua/plugin/clock.lua", CLOCK),
        ("init.lua", "require('plugin.clock')"),
    ]);
    write(dir.path(), "init.lua", "");
    sub.reload(None);
    assert!(sub.registry().snapshot(KindTag::Tick).is_empty());
}

/// A plugin that now fails is swept and reported, and the rest of the config still loads.
#[test]
fn a_plugin_that_now_fails_does_not_stop_the_rest_loading() {
    let (dir, sub) = built(&[
        ("lua/plugin/clock.lua", CLOCK),
        (
            "lua/plugin/other.lua",
            "neotmux.group('plugin.other').on_tick(function() end)",
        ),
        (
            "init.lua",
            "require('plugin.clock') require('plugin.other')",
        ),
    ]);
    write(
        dir.path(),
        "lua/plugin/clock.lua",
        "local g = neotmux.group('plugin.clock') g.on_tick(function() end) error('broken')",
    );
    let failures = sub.reload(None);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(
        sub.registry().snapshot(KindTag::Tick).len(),
        1,
        "only plugin.other's tick"
    );
}

/// A group named lazily, from a callback, is swept by a reload that does not name it.
#[test]
fn a_group_named_from_a_callback_is_swept_by_a_reload_that_does_not_name_it() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.keymap.set('C-x', function() neotmux.group('lazy').on_tick(function() end) return true end)",
    )]);
    sub.dispatch_key(support::one(), "C-x");
    assert_eq!(sub.registry().snapshot(KindTag::Tick).len(), 1);
    assert!(sub.reload(None).is_empty());
    assert!(sub.registry().snapshot(KindTag::Tick).is_empty());
}

#[test]
fn neotmux_reload_raises_the_op_and_does_not_reload_where_it_runs() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.keymap.set('C-r', function() neotmux.reload() return true end)",
    )]);
    sub.dispatch_key(support::one(), "C-r");
    let ops = sub.drain_ops(Some(support::one()));
    assert!(ops.iter().any(|(_, op)| *op == Op::Reload), "{ops:?}");
}

/// A tick the budget parked has left the registry, so no group can see it; without the
/// reload forgetting it, `resubscribe` would bring the old code's callback back ungrouped.
#[test]
fn a_reload_forgets_the_ticks_the_budget_parked() {
    let (dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 1 \
             require('plugin.spinner') require('plugin.rescuer')",
        ),
        (
            "lua/plugin/spinner.lua",
            "neotmux.group('plugin.spinner').on_tick(function() while true do end end)",
        ),
        (
            "lua/plugin/rescuer.lua",
            "neotmux.keymap.set('C-r', function()
               local handles = neotmux.resubscribe('plugin/spinner')
               neotmux.notify('info', handles and tostring(#handles) or 'nil')
               return true
             end)",
        ),
    ]);
    frame(&sub);
    assert!(sub.registry().snapshot(KindTag::Tick).is_empty(), "parked");
    sub.drain_ops(Some(support::one()));

    write(dir.path(), "init.lua", "require('plugin.rescuer')");
    assert!(sub.reload(None).is_empty());
    sub.dispatch_key(support::one(), "C-r");
    assert_eq!(notified(&sub), vec!["nil".to_string()]);
}

fn shipped_with_everything_on() -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "init.lua",
        "local d = require('neotmux.defaults')
         for k in pairs(d) do d[k] = true end",
    );
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), support::shipped_layer()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

/// The test the design rests on: a reload leaves what a start leaves.
#[test]
fn a_reload_of_the_shipped_runtime_leaves_what_a_start_leaves() {
    let (_dir, sub) = shipped_with_everything_on();
    let started = sub.census();
    assert!(sub.reload(None).is_empty());
    assert_eq!(sub.census(), started);
    assert!(sub.reload(None).is_empty());
    assert_eq!(sub.census(), started, "a second reload drifted");
}

#[test]
fn config_reload_is_a_command() {
    let (_dir, sub) = shipped_with_everything_on();
    assert!(sub.commands().iter().any(|c| c.name == "config-reload"));
}

use std::fs;

use neotmux_core::proto::{DrawCmd, Epoch, PaneId, Rect, Size};
use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::{Level, Op};
use neotmux_lua::registry::KindTag;
use neotmux_lua::runtime::Layers;

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn frame(sub: &LuaSubsystem) {
    let size = Size { rows: 24, cols: 80 };
    let pane = PaneId::from(1);
    sub.begin_frame(
        Epoch::new(size, vec![pane], Some(pane)),
        size,
        vec![(pane, Rect::new(0, 0, 80, 24))],
        Some(pane),
    );
    sub.dispatch_tick();
}

fn texts(sub: &LuaSubsystem) -> Vec<String> {
    sub.frame_output()
        .0
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn logs(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

const SPINNER: &str = "neotmux.on_tick(function() while true do end end) return {}";

#[test]
fn an_overrunning_tick_is_named_and_loses_its_frame() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 require('plugin.spinner')",
        ),
        (
            "lua/plugin/spinner.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'drawn') while true do end end) return {}",
        ),
    ]);

    frame(&sub);

    assert!(
        texts(&sub).is_empty(),
        "the overrunning tick's frame was kept"
    );
    let overruns: Vec<String> = logs(&sub)
        .into_iter()
        .filter(|text| text.contains("budget"))
        .collect();
    assert_eq!(overruns.len(), 1, "{overruns:?}");
    assert!(overruns[0].starts_with("plugin/spinner: "), "{overruns:?}");
}

#[test]
fn a_well_behaved_neighbour_still_draws_on_the_same_frame() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 require('plugin.spinner') require('plugin.painter')",
        ),
        ("lua/plugin/spinner.lua", SPINNER),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'alive') end) return {}",
        ),
    ]);

    frame(&sub);

    assert_eq!(texts(&sub), vec!["alive".to_string()]);
}

#[test]
fn an_ordinary_error_is_not_reported_as_an_overrun() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.bad')"),
        (
            "lua/plugin/bad.lua",
            "neotmux.on_tick(function() error('nope') end) return {}",
        ),
    ]);

    frame(&sub);

    let logs = logs(&sub);
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert!(logs[0].contains("nope"), "{logs:?}");
    assert!(!logs[0].contains("budget"), "{logs:?}");
    assert_eq!(sub.registry().snapshot(KindTag::Tick).len(), 1);
}

#[test]
fn an_overrun_is_reported_at_error_level() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 require('plugin.spinner')",
        ),
        ("lua/plugin/spinner.lua", SPINNER),
    ]);

    frame(&sub);

    let levels: Vec<Level> = sub
        .drain_ops()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { level, .. } => Some(level),
            _ => None,
        })
        .collect();
    assert_eq!(levels, vec![Level::Error]);
}

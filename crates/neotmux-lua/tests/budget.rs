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
            "neotmux.opt.tick_budget_ms = 5 require('plugin.probe') require('plugin.spinner')",
        ),
        (
            "lua/plugin/spinner.lua",
            "neotmux.on_tick(function(ctx)
               ctx:text(nil, 0, 0, 'drawn')
               while true do end
               _G.spinner_ran_past_its_loop = true
             end)
             return {}",
        ),
        (
            "lua/plugin/probe.lua",
            "_G.spinner_ran_past_its_loop = false
             neotmux.keymap.set('C-p', function()
               neotmux.notify('info', tostring(_G.spinner_ran_past_its_loop))
               return true
             end)
             return {}",
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

    // Without this the only symptom of a tick that was not cut off is a suite that hangs.
    sub.dispatch_key("C-p");
    assert_eq!(
        notifications(&sub),
        vec!["false".to_string()],
        "the tick ran past the loop the budget was supposed to cut off"
    );
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

fn notifications(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn tick_owners(sub: &LuaSubsystem) -> Vec<String> {
    sub.registry()
        .snapshot(KindTag::Tick)
        .into_iter()
        .filter_map(|id| sub.registry().owner(id))
        .filter_map(|owner| sub.sources().name(owner))
        .collect()
}

/// A spinner that also holds a keymap, so the unsubscribe can be shown to take the tick and
/// nothing else, and a control to prove the neighbour is untouched.
fn spinner_layer(extra_init: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "init.lua",
            format!(
                "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 3 \
                 require('plugin.spinner') {extra_init}"
            ),
        ),
        (
            "lua/plugin/spinner.lua",
            "neotmux.on_tick(function() while true do end end)
             neotmux.keymap.set('C-x', function() return true end)
             return {}"
                .to_string(),
        ),
    ]
}

fn built_owned(files: &[(&'static str, String)]) -> (tempfile::TempDir, LuaSubsystem) {
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, body)| (*rel, body.as_str()))
        .collect();
    built(&borrowed)
}

#[test]
fn a_repeat_offender_loses_its_tick_and_keeps_everything_else() {
    let (_dir, sub) = built_owned(&spinner_layer(""));

    for _ in 0..3 {
        frame(&sub);
    }

    assert!(tick_owners(&sub).is_empty(), "{:?}", tick_owners(&sub));
    let notes: Vec<String> = notifications(&sub)
        .into_iter()
        .filter(|text| text.contains("plugin/spinner"))
        .collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        sub.dispatch_key("C-x"),
        neotmux_lua::KeyOutcome::Consumed,
        "the unsubscribe took the keymap too"
    );
}

#[test]
fn the_last_frame_of_a_removed_offender_is_not_replayed() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 2 \
             require('plugin.flaky')",
        ),
        (
            "lua/plugin/flaky.lua",
            "local n = 0
             neotmux.on_tick(function(ctx)
               n = n + 1
               if n == 1 then ctx:text(nil, 0, 0, 'stale') return end
               while true do end
             end)
             return {}",
        ),
    ]);

    frame(&sub);
    assert_eq!(texts(&sub), vec!["stale".to_string()]);
    for _ in 0..2 {
        frame(&sub);
    }

    assert!(texts(&sub).is_empty(), "{:?}", texts(&sub));
}

#[test]
fn the_overrun_counter_resets_on_a_tick_within_budget() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 3 \
             require('plugin.flaky')",
        ),
        (
            "lua/plugin/flaky.lua",
            "local n = 0
             neotmux.on_tick(function()
               n = n + 1
               if n ~= 3 then while true do end end
             end)
             return {}",
        ),
    ]);

    for _ in 0..5 {
        frame(&sub);
    }

    assert_eq!(
        tick_owners(&sub),
        vec!["plugin/flaky".to_string()],
        "two runs of two overruns crossed a limit of three"
    );
}

#[test]
fn resubscribe_restores_a_removed_tick_and_it_draws_again() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 2 \
             require('plugin.flaky') require('plugin.rescuer')",
        ),
        (
            "lua/plugin/flaky.lua",
            "local rescued = false
             neotmux.on_tick(function(ctx)
               if rescued then ctx:text(nil, 0, 0, 'back') return end
               while true do end
             end)
             _G.flaky_rescued = function() rescued = true end
             return {}",
        ),
        (
            "lua/plugin/rescuer.lua",
            "neotmux.keymap.set('C-r', function()
               _G.flaky_rescued()
               neotmux.notify('info', tostring(neotmux.resubscribe('plugin/flaky')))
               return true
             end)
             return {}",
        ),
    ]);

    for _ in 0..2 {
        frame(&sub);
    }
    assert!(tick_owners(&sub).is_empty());
    sub.drain_ops();

    sub.dispatch_key("C-r");
    assert_eq!(notifications(&sub), vec!["true".to_string()]);

    frame(&sub);
    assert_eq!(texts(&sub), vec!["back".to_string()]);
}

#[test]
fn resubscribe_for_a_name_that_was_never_removed_is_false() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.rescuer')"),
        (
            "lua/plugin/rescuer.lua",
            "neotmux.on_tick(function() end)
             neotmux.keymap.set('C-r', function()
               neotmux.notify('info', tostring(neotmux.resubscribe('plugin/rescuer'))
                 .. ' ' .. tostring(neotmux.resubscribe('no/such/plugin')))
               return true
             end)
             return {}",
        ),
    ]);

    sub.dispatch_key("C-r");

    assert_eq!(notifications(&sub), vec!["false false".to_string()]);
    assert_eq!(tick_owners(&sub), vec!["plugin/rescuer".to_string()]);
}

#[test]
fn a_restored_tick_is_dropped_again_if_it_keeps_overrunning() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 2 \
             require('plugin.spinner') require('plugin.rescuer')",
        ),
        ("lua/plugin/spinner.lua", SPINNER),
        (
            "lua/plugin/rescuer.lua",
            "neotmux.keymap.set('C-r', function()
               neotmux.notify('info', tostring(neotmux.resubscribe('plugin/spinner')))
               return true
             end)
             return {}",
        ),
    ]);

    for _ in 0..2 {
        frame(&sub);
    }
    assert!(tick_owners(&sub).is_empty());
    sub.dispatch_key("C-r");
    assert_eq!(tick_owners(&sub), vec!["plugin/spinner".to_string()]);

    frame(&sub);
    assert_eq!(
        tick_owners(&sub),
        vec!["plugin/spinner".to_string()],
        "the restored tick was dropped by a single overrun, so the counter did not reset"
    );
    frame(&sub);
    assert!(tick_owners(&sub).is_empty());
}

/// A heavy but bounded tick: every non-zero budget in the suite would cut it off, so its
/// survival is the budget being off rather than the loop being quick.
const HEAVY: &str = "neotmux.on_tick(function(ctx)
     local n = 0
     for _ = 1, 20000000 do n = n + 1 end
     ctx:text(nil, 0, 0, 'heavy')
   end)
   return {}";

#[test]
fn a_budget_of_zero_measures_nothing_and_unsubscribes_nobody() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 0 neotmux.opt.tick_overrun_limit = 2 \
             require('plugin.heavy')",
        ),
        ("lua/plugin/heavy.lua", HEAVY),
    ]);

    for _ in 0..3 {
        frame(&sub);
        assert_eq!(texts(&sub), vec!["heavy".to_string()]);
        assert!(logs(&sub).is_empty());
    }

    assert_eq!(tick_owners(&sub), vec!["plugin/heavy".to_string()]);
}

/// The re-`require` of §5.3 is the recovery a plugin author reaches for instead of
/// `resubscribe`, and it must not leave the parked callbacks to be restored later.
#[test]
fn registering_a_tick_again_drops_what_was_parked_for_that_source() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 1 \
             require('plugin.spinner') require('plugin.rescuer')",
        ),
        ("lua/plugin/spinner.lua", SPINNER),
        (
            "lua/plugin/rescuer.lua",
            "neotmux.keymap.set('C-l', function()
               package.loaded['plugin.spinner'] = nil
               require('plugin.spinner')
               return true
             end)
             neotmux.keymap.set('C-r', function()
               neotmux.notify('info', tostring(neotmux.resubscribe('plugin/spinner')))
               return true
             end)
             return {}",
        ),
    ]);

    frame(&sub);
    assert!(tick_owners(&sub).is_empty());
    sub.drain_ops();

    sub.dispatch_key("C-l");
    assert_eq!(tick_owners(&sub), vec!["plugin/spinner".to_string()]);

    sub.dispatch_key("C-r");
    assert_eq!(notifications(&sub), vec!["false".to_string()]);
    assert_eq!(
        tick_owners(&sub),
        vec!["plugin/spinner".to_string()],
        "the parked callback was restored beside the freshly required one"
    );
}

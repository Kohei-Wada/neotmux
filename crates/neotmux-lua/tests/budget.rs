use std::fs;

use neotmux_lua::ops::Op;
use neotmux_lua::registry::KindTag;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{Geometry, LuaSubsystem};
use neotmux_proto::{DrawCmd, Epoch, Level, PaneId, Rect, Size};

mod support;

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn frame(sub: &LuaSubsystem) {
    let size = Size { rows: 24, cols: 80 };
    let pane = PaneId::from(1);
    sub.begin_frame(
        Epoch::new(size, vec![pane], Some(pane)),
        Geometry {
            size,
            panes: vec![(pane, Rect::new(0, 0, 80, 24))],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick();
}

fn texts(sub: &LuaSubsystem) -> Vec<String> {
    sub.frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn logs(sub: &LuaSubsystem) -> Vec<String> {
    logged(sub).into_iter().map(|(_, text)| text).collect()
}

/// The source a log op is attributed to, beside its text. Attribution lives in the op, not in
/// the text, so a test that cares which source was blamed has to read it here.
fn logged(sub: &LuaSubsystem) -> Vec<(String, String)> {
    sub.drain_ops()
        .into_iter()
        .filter_map(|(owner, op)| match op {
            Op::Log { text, .. } => Some((sub.sources().name(owner).unwrap_or_default(), text)),
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
    let overruns: Vec<(String, String)> = logged(&sub)
        .into_iter()
        .filter(|(_, text)| text.contains("budget"))
        .collect();
    assert_eq!(overruns.len(), 1, "{overruns:?}");
    assert_eq!(overruns[0].0, "plugin/spinner", "{overruns:?}");

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
               local handles = neotmux.resubscribe('plugin/flaky')
               neotmux.notify('info', handles and tostring(#handles) or 'nil')
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
    assert_eq!(notifications(&sub), vec!["1".to_string()]);

    frame(&sub);
    assert_eq!(texts(&sub), vec!["back".to_string()]);
}

#[test]
fn resubscribe_for_a_name_that_was_never_removed_is_nil() {
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

    assert_eq!(notifications(&sub), vec!["nil nil".to_string()]);
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
               local handles = neotmux.resubscribe('plugin/spinner')
               neotmux.notify('info', handles and tostring(#handles) or 'nil')
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
               local handles = neotmux.resubscribe('plugin/spinner')
               neotmux.notify('info', handles and tostring(#handles) or 'nil')
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
    assert_eq!(notifications(&sub), vec!["nil".to_string()]);
    assert_eq!(
        tick_owners(&sub),
        vec!["plugin/spinner".to_string()],
        "the parked callback was restored beside the freshly required one"
    );
}

fn built_layers(layers: &[&[(&str, &str)]]) -> (Vec<tempfile::TempDir>, LuaSubsystem) {
    let mut dirs = Vec::new();
    for files in layers {
        let dir = tempfile::tempdir().unwrap();
        for (rel, body) in *files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        dirs.push(dir);
    }
    let paths = dirs.iter().map(|d| d.path().to_path_buf()).collect();
    let sub = LuaSubsystem::build(Layers::from_paths(paths), support::panes()).unwrap();
    assert!(sub.run_init().is_empty());
    (dirs, sub)
}

/// Two layers' `init.lua` files share the display name `init`, so `resubscribe` has an
/// ambiguous handle. The first interned match — the earlier layer — wins, deliberately.
#[test]
fn resubscribe_restores_the_first_source_interned_under_a_shared_name() {
    // Each layer draws its tag before spinning, and the spin is switched off once one of them
    // is back: a discarded frame keeps neither drawing nor ops, so the tag has to come from a
    // frame that completes.
    let spinner = |tag: &str| {
        format!(
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 1
             _G.spin = true
             neotmux.on_tick(function(ctx)
               ctx:text(nil, 0, 0, '{tag}')
               if _G.spin then while true do end end
             end)"
        )
    };
    let first = format!(
        "{}
         neotmux.keymap.set('C-r', function()
           local handles = neotmux.resubscribe('init')
           neotmux.notify('info', handles and tostring(#handles) or 'nil')
           _G.spin = false
           return true
         end)",
        spinner("first")
    );
    let second = spinner("second");
    let (_dirs, sub) = built_layers(&[
        &[("init.lua", first.as_str())],
        &[("init.lua", second.as_str())],
    ]);

    frame(&sub);
    assert!(tick_owners(&sub).is_empty());
    sub.drain_ops();

    sub.dispatch_key("C-r");
    assert_eq!(notifications(&sub), vec!["1".to_string()]);

    frame(&sub);
    assert_eq!(texts(&sub), vec!["first".to_string()]);
}

/// §5.4's `pcall` hole: the budget error is catchable, so the tick returns normally. What it
/// does not buy the plugin is the frame — or the count against its limit.
#[test]
fn a_tick_that_pcalls_away_the_budget_error_still_loses_its_frame() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 2 \
             require('plugin.swallower')",
        ),
        (
            "lua/plugin/swallower.lua",
            "neotmux.on_tick(function(ctx)
               ctx:text(nil, 0, 0, 'swallowed')
               for _ = 1, 2000 do pcall(function() while true do end end) end
             end)
             return {}",
        ),
    ]);

    frame(&sub);

    assert!(texts(&sub).is_empty(), "{:?}", texts(&sub));
    let overruns: Vec<String> = logs(&sub)
        .into_iter()
        .filter(|text| text.contains("budget"))
        .collect();
    assert_eq!(overruns.len(), 1, "{overruns:?}");

    frame(&sub);
    assert!(
        tick_owners(&sub).is_empty(),
        "a swallowed overrun was not counted"
    );
}

/// One `string.rep` and nothing else: 8 MiB measures ~30 ms here against a 5 ms budget, the
/// smallest size whose margin survives a much faster machine without slowing the suite.
const HOG: &str = "neotmux.on_tick(function(ctx)
     ctx:text(nil, 0, 0, 'hogged')
     local s = string.rep('x', 8 * 1024 * 1024)
     _G.hog_finished = #s
   end)
   return {}";

const HOG_INIT: &str =
    "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 2 require('plugin.hog') ";

/// §5.4's first hole: the hook never gets a sample past the deadline, so the tick cannot be
/// cut off — but the elapsed check at the end of the group still names it.
#[test]
fn a_long_c_call_is_reported_by_name_though_it_ran_to_completion() {
    let (_dir, sub) = built(&[
        ("init.lua", &format!("{HOG_INIT} require('plugin.probe')")),
        ("lua/plugin/hog.lua", HOG),
        (
            "lua/plugin/probe.lua",
            "_G.hog_finished = 0
             neotmux.keymap.set('C-p', function()
               neotmux.notify('info', tostring(_G.hog_finished))
               return true
             end)
             return {}",
        ),
    ]);

    frame(&sub);

    let overruns: Vec<(String, String)> = logged(&sub)
        .into_iter()
        .filter(|(_, text)| text.contains("budget"))
        .collect();
    assert_eq!(overruns.len(), 1, "{overruns:?}");
    assert_eq!(overruns[0].0, "plugin/hog", "{overruns:?}");
    assert!(
        overruns[0].1.contains("not cut off"),
        "an uninterrupted overrun read as an interrupted one: {overruns:?}"
    );
    // The report says what was observed — no sample past the deadline — not where the time
    // went, and says it with enough precision that an overrun reads as one.
    assert!(
        overruns[0].1.contains("no sample landed past the deadline"),
        "{overruns:?}"
    );
    assert!(
        overruns[0].1.contains('.'),
        "an overrun was rendered with no fractional milliseconds: {overruns:?}"
    );

    // The distinguishing fact: nothing was interrupted, the tick ran its C call to the end.
    sub.dispatch_key("C-p");
    assert_eq!(notifications(&sub), vec!["8388608".to_string()]);
}

#[test]
fn a_long_c_call_loses_its_frame() {
    let (_dir, sub) = built(&[("init.lua", HOG_INIT), ("lua/plugin/hog.lua", HOG)]);

    frame(&sub);

    assert!(texts(&sub).is_empty(), "{:?}", texts(&sub));
}

#[test]
fn a_long_c_call_every_frame_parks_the_source_and_resubscribe_brings_it_back() {
    let (_dir, sub) = built(&[
        ("init.lua", &format!("{HOG_INIT} require('plugin.rescuer')")),
        ("lua/plugin/hog.lua", HOG),
        (
            "lua/plugin/rescuer.lua",
            "neotmux.keymap.set('C-r', function()
               local handles = neotmux.resubscribe('plugin/hog')
               neotmux.notify('info', handles and tostring(#handles) or 'nil')
               return true
             end)
             return {}",
        ),
    ]);

    for _ in 0..2 {
        frame(&sub);
    }
    assert!(tick_owners(&sub).is_empty(), "{:?}", tick_owners(&sub));
    sub.drain_ops();

    sub.dispatch_key("C-r");
    assert_eq!(notifications(&sub), vec!["1".to_string()]);
    assert_eq!(tick_owners(&sub), vec!["plugin/hog".to_string()]);
}

#[test]
fn a_fast_tick_with_no_hook_sample_is_not_reported() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 2 \
             require('plugin.quick')",
        ),
        (
            "lua/plugin/quick.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'quick') end) return {}",
        ),
    ]);

    for _ in 0..3 {
        frame(&sub);
        assert!(
            logs(&sub).is_empty(),
            "a tick inside its budget was reported"
        );
        assert_eq!(texts(&sub), vec!["quick".to_string()]);
    }

    assert_eq!(tick_owners(&sub), vec!["plugin/quick".to_string()]);
}

#[test]
fn a_budget_of_zero_says_nothing_about_a_long_c_call() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 0 neotmux.opt.tick_overrun_limit = 2 \
             require('plugin.hog')",
        ),
        ("lua/plugin/hog.lua", HOG),
    ]);

    for _ in 0..3 {
        frame(&sub);
        assert_eq!(texts(&sub), vec!["hogged".to_string()]);
        assert!(logs(&sub).is_empty(), "{:?}", logs(&sub));
    }

    assert_eq!(tick_owners(&sub), vec!["plugin/hog".to_string()]);
}

/// §5.1's "the host discards that plugin's command buffer": the drawing is only half of it.
/// A tick that splits the window and then spins would otherwise split it once per frame right
/// up to the park.
#[test]
fn an_overrunning_tick_loses_the_ops_it_pushed_as_well_as_its_drawing() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 require('plugin.neighbour') \
             require('plugin.spinner')",
        ),
        (
            "lua/plugin/spinner.lua",
            "neotmux.on_tick(function(ctx)
               ctx:text(nil, 0, 0, 'drawn')
               neotmux.pane.split('horizontal')
               neotmux.notify('info', 'splitting')
               while true do end
             end)
             return {}",
        ),
        (
            "lua/plugin/neighbour.lua",
            "neotmux.on_tick(function() neotmux.notify('info', 'neighbour') end) return {}",
        ),
    ]);

    frame(&sub);

    let ops = sub.drain_ops();
    assert!(
        !ops.iter().any(|(_, op)| matches!(op, Op::PaneSplit { .. })),
        "the discarded frame still split the window: {ops:?}"
    );
    let notes: Vec<&str> = ops
        .iter()
        .filter_map(|(_, op)| match op {
            Op::Notify { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes,
        vec!["neighbour"],
        "the discard took a notification it did not push"
    );
    // The overrun report is pushed after the discard, so the discard must not eat it.
    let overruns: Vec<&str> = ops
        .iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } if text.contains("budget") => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(overruns.len(), 1, "{ops:?}");
}

/// A tick that raises loses its frame, and the ops it pushed on the way to raising go with it
/// — but the error the host reports for it does not.
#[test]
fn a_raising_tick_loses_its_ops_but_is_still_reported() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.bad')"),
        (
            "lua/plugin/bad.lua",
            "neotmux.on_tick(function()
               neotmux.pane.split('vertical')
               error('nope')
             end)
             return {}",
        ),
    ]);

    frame(&sub);

    let ops = sub.drain_ops();
    assert!(
        !ops.iter().any(|(_, op)| matches!(op, Op::PaneSplit { .. })),
        "{ops:?}"
    );
    let logs: Vec<&str> = ops
        .iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(logs.len(), 1, "{ops:?}");
    assert!(logs[0].contains("nope"), "{logs:?}");
}

/// A restored registration is a new one, with an id the caller's old handle does not name.
/// Without handles back from `resubscribe` there is no way to remove it again at all.
#[test]
fn the_handles_resubscribe_returns_remove_the_registrations_it_restored() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 1 \
             require('plugin.spinner') require('plugin.rescuer')",
        ),
        ("lua/plugin/spinner.lua", SPINNER),
        (
            "lua/plugin/rescuer.lua",
            "local handles
             neotmux.keymap.set('C-r', function()
               handles = neotmux.resubscribe('plugin/spinner')
               return true
             end)
             neotmux.keymap.set('C-x', function()
               for _, handle in ipairs(handles) do handle:remove() end
               return true
             end)
             return {}",
        ),
    ]);

    frame(&sub);
    assert!(tick_owners(&sub).is_empty());

    sub.dispatch_key("C-r");
    assert_eq!(tick_owners(&sub), vec!["plugin/spinner".to_string()]);

    sub.dispatch_key("C-x");
    assert!(
        tick_owners(&sub).is_empty(),
        "the handle resubscribe returned did not name the restored registration: {:?}",
        tick_owners(&sub)
    );
}

/// One handle per restored registration, not one per source: a file with two ticks parks both
/// and has to be able to remove both.
#[test]
fn resubscribe_returns_one_handle_for_each_tick_it_restored() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 1 \
             require('plugin.twin') require('plugin.rescuer')",
        ),
        (
            "lua/plugin/twin.lua",
            "neotmux.on_tick(function() end)
             neotmux.on_tick(function() while true do end end)
             return {}",
        ),
        (
            "lua/plugin/rescuer.lua",
            "neotmux.keymap.set('C-r', function()
               local handles = neotmux.resubscribe('plugin/twin')
               neotmux.notify('info', handles and tostring(#handles) or 'nil')
               return true
             end)
             return {}",
        ),
    ]);

    frame(&sub);
    assert!(tick_owners(&sub).is_empty());
    sub.drain_ops();

    sub.dispatch_key("C-r");

    assert_eq!(notifications(&sub), vec!["2".to_string()]);
    assert_eq!(tick_owners(&sub).len(), 2);
}

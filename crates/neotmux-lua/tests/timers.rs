//! `neotmux.timer`: a plugin asks to be called again later. The clock is the test's — every
//! frame is handed the instant it happens at, the way `PaneInfoCache::refresh` already is.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 10, cols: 40 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// The shipped layer with every plugin off, so a frame holds nothing but the test's own
/// timers. No runner: nothing here starts a process.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'sync', 'hints' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
    (dir, Built::new(sub, terminals))
}

fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn frame(sub: &LuaSubsystem, at: Instant) {
    sub.dispatch_timers(support::one(), at);
    sub.dispatch_tick(support::one());
}

/// `every` fires each time its interval has passed, and goes on firing.
#[test]
fn every_fires_once_its_interval_has_passed_and_keeps_firing() {
    let (_dir, sub) = built("neotmux.timer.every(100, function() neotmux.log.info('fired') end)\n");
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(16));
    assert!(logged(&sub).is_empty(), "fired before its interval");

    frame(&sub, start + Duration::from_millis(120));
    assert_eq!(logged(&sub), vec!["fired"]);

    frame(&sub, start + Duration::from_millis(180));
    assert!(logged(&sub).is_empty(), "fired twice in one interval");

    frame(&sub, start + Duration::from_millis(240));
    assert_eq!(logged(&sub), vec!["fired"]);
}

/// A timer armed while an `init.lua` runs — before any session exists — is adopted by the
/// first session to run a frame, and fires there. This is the ordinary case for a plugin that
/// arms a poll at load, so it is not left to the other tests to cover by accident.
#[test]
fn a_timer_armed_before_any_session_exists_fires_in_the_first_frame_that_comes() {
    let (_dir, sub) = built("neotmux.timer.every(10, function() neotmux.log.info('fired') end)\n");
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(50));
    assert_eq!(logged(&sub), vec!["fired"]);
}

/// A timer that should have fired five times while nothing was ticking fires once. There is
/// no catch-up: a plugin that cares how much time passed asks the clock, and one that does
/// not would rather be called once than five times in a row.
#[test]
fn a_timer_that_missed_several_intervals_fires_once() {
    let (_dir, sub) = built("neotmux.timer.every(100, function() neotmux.log.info('fired') end)\n");
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(1000));
    assert_eq!(logged(&sub), vec!["fired"]);
    frame(&sub, start + Duration::from_millis(1016));
    assert!(
        logged(&sub).is_empty(),
        "the missed intervals were caught up"
    );
}

/// `after` fires once and is then done, whatever happens next.
#[test]
fn after_fires_once_and_stops() {
    let (_dir, sub) = built(
        "local t = neotmux.timer.after(50, function() neotmux.log.info('fired') end)\n\
         neotmux.on_key(function(key)\n\
           if key == 'S' then\n\
             neotmux.log.info('stop=' .. tostring(t:stop()))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(60));
    assert_eq!(logged(&sub), vec!["fired"]);

    frame(&sub, start + Duration::from_millis(200));
    assert!(logged(&sub).is_empty(), "a one-shot fired twice");

    sub.dispatch_key(support::one(), "S");
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "stop=false"),
        "stopping a timer that already fired claimed to stop something: {said:?}"
    );
}

/// A timer stopped from inside its own callback does not fire again.
#[test]
fn a_timer_can_stop_itself() {
    let (_dir, sub) = built(
        "local t\n\
         t = neotmux.timer.every(10, function()\n\
           neotmux.log.info('fired')\n\
           t:stop()\n\
         end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(20));
    assert_eq!(logged(&sub), vec!["fired"]);
    frame(&sub, start + Duration::from_millis(400));
    assert!(
        logged(&sub).is_empty(),
        "a timer that stopped itself fired again"
    );
}

/// An interval below a frame is a frame: asking for 1ms means "every frame", and is not a
/// promise the host could keep any other way.
#[test]
fn an_interval_below_a_frame_fires_every_frame() {
    let (_dir, sub) = built("neotmux.timer.every(1, function() neotmux.log.info('fired') end)\n");
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(16));
    assert_eq!(logged(&sub), vec!["fired"]);
    frame(&sub, start + Duration::from_millis(32));
    assert_eq!(logged(&sub), vec!["fired"]);
}

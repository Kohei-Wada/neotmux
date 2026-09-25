//! `neotmux.timer`: a plugin asks to be called again later. The clock is the test's — every
//! frame is handed the instant it happens at, the way `PaneInfoCache::refresh` already is.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, SessionId, Size};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 10, cols: 40 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
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

/// The first frame of a session other than the one `built` already began.
fn begin(sub: &LuaSubsystem, session: SessionId) {
    sub.begin_frame(
        session,
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
}

fn logged(sub: &Built) -> Vec<String> {
    logged_in(sub, support::one())
}

fn logged_in(sub: &Built, session: SessionId) -> Vec<String> {
    sub.drain_ops(Some(session))
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

/// Adoption is a handover, not a broadcast: once a session has picked a timer up it is that
/// session's, and every other session's frame walks past it. The session that arms nothing is
/// the one that must never hear it — an implementation where any session fires any timer
/// passes the single-session test above and fails this one.
#[test]
fn a_session_that_did_not_adopt_a_timer_never_sees_it() {
    let two = SessionId::from(2);
    let (_dir, sub) = built("neotmux.timer.every(10, function() neotmux.log.info('fired') end)\n");
    begin(&sub, two);
    let start = Instant::now();

    // Session two's frame gets there first, so the timer is two's from here on.
    sub.dispatch_timers(two, start + Duration::from_millis(50));
    assert_eq!(logged_in(&sub, two), vec!["fired"]);

    sub.dispatch_timers(support::one(), start + Duration::from_millis(500));
    assert!(
        logged_in(&sub, support::one()).is_empty(),
        "session one heard a timer it never adopted"
    );

    sub.dispatch_timers(two, start + Duration::from_millis(500));
    assert_eq!(logged_in(&sub, two), vec!["fired"]);
}

/// Two one-shots come due in the same frame and the first cancels the second. The second's
/// callback must not run — and `stop` must say so, because a plugin reads `false` as "too
/// late, it already ran", which is the opposite of what happened.
#[test]
fn stopping_a_due_timer_before_it_is_called_cancels_it_and_says_so() {
    let (_dir, sub) = built(
        "local second\n\
         neotmux.timer.after(50, function()\n\
           neotmux.log.info('first')\n\
           neotmux.log.info('stop=' .. tostring(second:stop()))\n\
         end)\n\
         second = neotmux.timer.after(50, function() neotmux.log.info('second') end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(60));
    let said = logged(&sub);
    assert!(
        !said.iter().any(|line| line == "second"),
        "a timer stopped before it was called ran anyway: {said:?}"
    );
    assert!(
        said.iter().any(|line| line == "stop=true"),
        "cancelling a call that then did not happen answered false: {said:?}"
    );
}

/// Stopping an `after` before its deadline answers true, and it never fires.
#[test]
fn stopping_a_one_shot_before_it_fires_answers_true() {
    let (_dir, sub) = built(
        "local t = neotmux.timer.after(100, function() neotmux.log.info('fired') end)\n\
         neotmux.on_key(function(key)\n\
           if key == 'S' then\n\
             neotmux.log.info('stop=' .. tostring(t:stop()))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    let start = Instant::now();

    sub.dispatch_key(support::one(), "S");
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "stop=true"), "{said:?}");
    frame(&sub, start + Duration::from_millis(400));
    assert!(logged(&sub).is_empty(), "a stopped one-shot fired");
}

/// Zero is the same ask as one: there is nothing shorter than a frame to fire on, so it is a
/// frame.
#[test]
fn a_zero_interval_is_clamped_to_the_frame() {
    let (_dir, sub) = built("neotmux.timer.every(0, function() neotmux.log.info('fired') end)\n");
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(8));
    assert!(logged(&sub).is_empty(), "fired inside the first frame");
    frame(&sub, start + Duration::from_millis(16));
    assert_eq!(logged(&sub), vec!["fired"]);
}

/// A timer whose callback keeps overrunning is stopped — and only it. The source's drawing is
/// not the thing that misbehaved, so the source's drawing is left alone.
#[test]
fn a_timer_that_keeps_overrunning_is_stopped_and_the_tick_is_not() {
    let (_dir, sub) = built(
        "neotmux.opt.tick_budget_ms = 1\n\
         neotmux.opt.tick_overrun_limit = 2\n\
         neotmux.timer.every(10, function()\n\
           neotmux.log.info('timer')\n\
           local t = os.clock()\n\
           while os.clock() - t < 0.05 do end\n\
         end)\n\
         neotmux.on_tick(function() neotmux.log.info('tick') end)\n",
    );
    let start = Instant::now();

    // Per frame, because the tick shares the timer's 1 ms budget: a frame the timer spun in
    // may lose its own `tick` line to the overrun that spinning caused, and counting the
    // whole run would make this test ride on that. What it is here to prove is that the
    // frames *after* the timer is gone still tick.
    let mut per_frame = Vec::new();
    for step in 1..=4 {
        frame(&sub, start + Duration::from_millis(100 * step));
        per_frame.push(logged(&sub));
    }
    let said: Vec<&String> = per_frame.iter().flatten().collect();
    assert_eq!(
        said.iter().filter(|line| **line == "timer").count(),
        2,
        "the timer was not stopped after two overruns: {said:?}"
    );
    for (at, frame) in per_frame.iter().enumerate().skip(2) {
        assert!(
            frame.iter().any(|line| line == "tick"),
            "stopping the timer took the source's tick with it, from frame {}: {per_frame:?}",
            at + 1
        );
    }
}

/// A session that is gone takes its timers with it, including a one-shot that has already
/// fired and is lingering for the frame it fired in. Nothing is left to fire into a session
/// nobody will draw again.
#[test]
fn forgetting_a_session_stops_the_timers_it_adopted() {
    let (_dir, sub) = built(
        "neotmux.timer.after(50, function() neotmux.log.info('once') end)\n\
         neotmux.timer.every(50, function() neotmux.log.info('again') end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(60));
    assert_eq!(logged(&sub), vec!["once", "again"]);

    sub.forget_session(support::one());
    frame(&sub, start + Duration::from_millis(400));
    assert!(
        logged(&sub).is_empty(),
        "a forgotten session's timers went on firing"
    );
}

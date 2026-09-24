//! `plugin.chime`: a pane that starts waiting for you makes a noise, once, on a player the
//! machine actually has.
//!
//! The panes are driven with the captured Claude Code screens `support` holds, so the path
//! under test is the real one — the ruleset reads a screen, `neotmux.state` answers, and the
//! plugin compares that answer against what it last saw. Nothing here stubs `state.of`.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::runner::JobChunk;
use neotmux_lua::runtime::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};

mod support;

use support::{Built, FakeRunner, notified, permission_screen, working_screen};

/// The captures' own size. Wide enough that none of Claude Code's rules wraps onto a second
/// line, tall enough that the seventeen-line permission screen does not scroll off its top.
const SIZE: Size = Size {
    rows: 40,
    cols: 100,
};

/// The two panes the harness publishes geometry for. `ONE` is focused, so `TWO` is the pane a
/// test can make wait without the focus rule silencing it.
const ONE: u32 = 1;
const TWO: u32 = 2;

/// Every shipped plugin off but `chime`, which is on by default and stays on: what is under
/// test is the shipped default, not a plugin the test turned on for itself.
const TWO_PANES: &str = "local defaults = require('neotmux.defaults')\n\
     for _, name in ipairs({ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
       'notify', 'borders', 'guard', 'sync', 'watch', 'sidebar', 'footer',\n\
       'hints' }) do\n\
       defaults[name] = false\n\
     end\n";

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

fn built(init: &str, runner: Rc<FakeRunner>) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        Rc::new(neotmux_lua::clipboard::Absent),
        runner,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    let panes = vec![PaneId::from(ONE), PaneId::from(TWO)];
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, panes.clone(), Some(PaneId::from(ONE))),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![
                (PaneId::from(ONE), Rect::new(0, 0, SIZE.cols / 2, SIZE.rows)),
                (
                    PaneId::from(TWO),
                    Rect::new(SIZE.cols / 2, 0, SIZE.cols / 2, SIZE.rows),
                ),
            ],
            focus: Some(PaneId::from(ONE)),
            ..Default::default()
        },
    );
    for pane in panes {
        terminals.open(pane, SIZE, 100);
    }
    (dir, Built::new(sub, terminals))
}

/// One frame: deliver whatever the jobs have said, then tick.
fn frame(sub: &LuaSubsystem) {
    sub.dispatch_jobs(support::one());
    sub.dispatch_tick(support::one());
}

/// A screen, put up in place of whatever was there. Cleared rather than re-opened: opening a
/// pane again would start its revision over, and the detector caches a look against that.
fn show(sub: &Built, pane: u32, screen: &str) {
    let pane = PaneId::from(pane);
    sub.terminals().feed(pane, b"\x1b[2J\x1b[H");
    sub.terminals()
        .feed(pane, screen.replace('\n', "\r\n").as_bytes());
}

fn blocked(sub: &Built, pane: u32) {
    show(sub, pane, &permission_screen());
}

fn working(sub: &Built, pane: u32) {
    show(sub, pane, &working_screen());
}

/// One sound per entry into a state: a pane that is still blocked next frame does not chime
/// again, and one that goes back to working and blocks again does.
#[test]
fn a_pane_chimes_when_it_starts_waiting_and_not_again() {
    let runner = FakeRunner::new();
    // Two panes, so the one under test is never the focused one.
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));
    blocked(&sub, TWO);
    frame(&sub);
    assert_eq!(
        runner.started.borrow().len(),
        1,
        "no sound for a pane that started waiting"
    );
    assert!(
        runner.started.borrow()[0]
            .cmd
            .last()
            .unwrap()
            .ends_with("request.mp3")
    );

    frame(&sub);
    assert_eq!(
        runner.started.borrow().len(),
        1,
        "it chimed again for the same wait"
    );

    working(&sub, TWO);
    frame(&sub);
    blocked(&sub, TWO);
    frame(&sub);
    assert_eq!(
        runner.started.borrow().len(),
        2,
        "a second wait is a second sound"
    );
}

/// The pane you are looking at does not make a noise at you.
#[test]
fn the_focused_pane_is_silent_by_default() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));
    blocked(&sub, ONE); // ONE is the focused pane in the harness's geometry
    frame(&sub);
    assert!(
        runner.started.borrow().is_empty(),
        "the pane in front of you chimed"
    );
}

/// A player is judged by how it exits: one that will not start is skipped, one that starts and
/// exits non-zero is skipped, and the first that exits zero is remembered — the ones before it
/// are not tried again.
#[test]
fn the_first_player_that_exits_zero_is_the_one_from_then_on() {
    let runner = FakeRunner::new();
    runner.refuse_program("paplay"); // not on this machine
    runner.will_say(vec![vec![JobChunk::Exited(1)]]); // pw-play: starts, cannot decode
    runner.will_say(vec![vec![JobChunk::Exited(0)]]); // ffplay: works
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));

    blocked(&sub, TWO);
    frame(&sub); // pw-play starts and fails
    frame(&sub); // its on_exit moves on to ffplay
    frame(&sub);
    let tried: Vec<String> = runner
        .started
        .borrow()
        .iter()
        .map(|spec| spec.cmd[0].clone())
        .collect();
    assert_eq!(tried, vec!["pw-play", "ffplay"], "{tried:?}");

    // The next chime goes straight to the one that worked.
    working(&sub, TWO);
    frame(&sub);
    blocked(&sub, TWO);
    frame(&sub);
    let tried: Vec<String> = runner
        .started
        .borrow()
        .iter()
        .map(|spec| spec.cmd[0].clone())
        .collect();
    assert_eq!(tried, vec!["pw-play", "ffplay", "ffplay"], "{tried:?}");
}

/// When none of them work, it says so once and stops.
#[test]
fn a_machine_with_no_player_is_told_once() {
    let runner = FakeRunner::new();
    runner.refuse_all();
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));

    blocked(&sub, TWO);
    frame(&sub);
    let said = notified(&sub);
    assert_eq!(
        said.iter()
            .filter(|line| line.contains("no player"))
            .count(),
        1,
        "{said:?}"
    );

    working(&sub, TWO);
    frame(&sub);
    blocked(&sub, TWO);
    frame(&sub);
    assert!(
        notified(&sub)
            .iter()
            .all(|line| !line.contains("no player")),
        "it kept trying after it had given up"
    );
}

/// A shell is not an agent. `neotmux.state` answers for one out of its pattern fallback, and a
/// shell reaches `idle` at the end of every command it runs — a sound on that edge would chime
/// for an `ls` in a pane nobody is looking at.
#[test]
fn a_shell_that_finishes_a_command_is_silent() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &format!("{TWO_PANES}require('neotmux.state').working_for = 0\n"),
        Rc::clone(&runner),
    );
    show(&sub, TWO, "$ ls -la /etc\ntotal 0\n$ ");
    frame(&sub);
    frame(&sub);
    assert!(
        runner.started.borrow().is_empty(),
        "an ordinary shell chimed at the end of a command"
    );
}

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
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size, Window, WindowId};

mod support;

use support::{
    Built, FakeRunner, idle_screen, logged, notified, permission_screen, working_screen,
};

/// The captures' own size. Wide enough that none of Claude Code's rules wraps onto a second
/// line, tall enough that the seventeen-line permission screen does not scroll off its top.
const SIZE: Size = Size {
    rows: 40,
    cols: 100,
};

/// The panes the harness publishes. `ONE` and `TWO` are the shown window's, with `ONE`
/// focused, so `TWO` is the pane a test can make wait without the focus rule silencing it.
/// `THREE` is in a second, hidden window — the pane you have walked away from.
const ONE: u32 = 1;
const TWO: u32 = 2;
const THREE: u32 = 3;

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
    let shown = vec![PaneId::from(ONE), PaneId::from(TWO)];
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, shown.clone(), Some(PaneId::from(ONE))),
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
            // Two windows, only one of them shown: `pane.list` is the shown one's panes, and a
            // plugin that watched only those would never hear from `THREE`.
            windows: vec![
                Window {
                    id: WindowId::from(1),
                    panes: shown.clone(),
                    name: None,
                },
                Window {
                    id: WindowId::from(2),
                    panes: vec![PaneId::from(THREE)],
                    name: None,
                },
            ],
            window: Some(WindowId::from(1)),
        },
    );
    for pane in [PaneId::from(ONE), PaneId::from(TWO), PaneId::from(THREE)] {
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

fn idle(sub: &Built, pane: u32) {
    show(sub, pane, &idle_screen());
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

/// The other half of the feature: a turn that finishes is a pane you can stop waiting for, and
/// it has a sound of its own. The prompt screen is `visible = 'idle'`, so the detector's hold
/// lets it through at once rather than sitting on it for `settle`.
#[test]
fn a_turn_that_finishes_chimes_the_other_sound() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));
    working(&sub, TWO);
    frame(&sub);
    assert!(
        runner.started.borrow().is_empty(),
        "a turn starting made a noise"
    );

    idle(&sub, TWO);
    frame(&sub);
    let started = runner.started.borrow();
    assert_eq!(started.len(), 1, "a turn that finished made no noise");
    assert!(
        started[0].cmd.last().unwrap().ends_with("done.mp3"),
        "{:?}",
        started[0].cmd
    );
}

/// Another window is exactly where you are when you have stopped watching the one an agent is
/// in, so the walk is every window's panes and not `pane.list`'s.
#[test]
fn a_pane_in_a_window_you_are_not_looking_at_chimes() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));
    blocked(&sub, THREE); // the hidden window's pane
    frame(&sub);
    let started = runner.started.borrow();
    assert_eq!(
        started.len(),
        1,
        "a pane in a window nobody is looking at waited in silence"
    );
    assert!(
        started[0].cmd.last().unwrap().ends_with("request.mp3"),
        "{:?}",
        started[0].cmd
    );
}

/// A killed job is not a verdict on the player. `on_exit` answers a nil code for every ending
/// that was not the program's own — a session sweep, the output limit — and reading one as
/// "this program does not work" would write off a player that works, for the life of the
/// server.
#[test]
fn a_player_killed_by_the_output_limit_is_not_written_off() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(vec![b'x'; 64])]]);
    let (_dir, sub) = built(
        &format!("{TWO_PANES}neotmux.opt.job_output_bytes = 16\n"),
        Rc::clone(&runner),
    );

    blocked(&sub, TWO);
    frame(&sub); // paplay starts
    frame(&sub); // it says more than it is allowed to and is killed: on_exit(nil, ...)

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
    assert_eq!(
        tried,
        vec!["paplay", "paplay"],
        "a killed job moved the walk on: {tried:?}"
    );
}

/// Nor is a refusal that never reached the program. `job.start` refuses when the source already
/// has `job_concurrency` jobs running, and a chime that read that as "paplay is not installed"
/// would walk to the end of the list and give up on a machine that plays sounds perfectly well.
#[test]
fn a_refusal_that_is_not_about_the_program_does_not_move_the_walk_on() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &format!("{TWO_PANES}neotmux.opt.job_concurrency = 1\n"),
        Rc::clone(&runner),
    );

    // The first chime takes the one slot there is, and nothing ever ends it.
    blocked(&sub, TWO);
    frame(&sub);
    blocked(&sub, THREE);
    frame(&sub);

    let tried: Vec<String> = runner
        .started
        .borrow()
        .iter()
        .map(|spec| spec.cmd[0].clone())
        .collect();
    assert_eq!(
        tried,
        vec!["paplay"],
        "a refusal about the job limit was read as a verdict on the player: {tried:?}"
    );
    assert!(
        notified(&sub)
            .iter()
            .all(|line| !line.contains("no player")),
        "it gave up on a machine whose player it had never actually tried"
    );
}

/// A shell is not an agent. `neotmux.state` answers for one out of its pattern fallback, and a
/// shell's trail is `working -> idle` at the end of every command it runs — `done.mp3` on that
/// edge would chime for an `ls` in a pane nobody is looking at. The trail is what the log line
/// pins: without it this would only be saying that an idle shell on first sight is silent.
#[test]
fn a_shell_that_finishes_a_command_is_silent() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &format!(
            "{TWO_PANES}require('neotmux.state').working_for = 100\n\
             local state = require('neotmux.state')\n\
             local last\n\
             neotmux.on_tick(function()\n\
               local now = tostring(state.of(2))\n\
               if now ~= last then\n\
                 neotmux.log.info('two=' .. now)\n\
                 last = now\n\
               end\n\
             end)\n"
        ),
        Rc::clone(&runner),
    );

    show(&sub, TWO, "$ ls -la /etc\ntotal 0\n");
    frame(&sub);
    // Past the fallback's window, with nothing new said, the same screen reads idle.
    std::thread::sleep(std::time::Duration::from_millis(150));
    frame(&sub);

    let said = logged(&sub);
    assert!(
        said.iter().any(|l| l == "two=working") && said.iter().any(|l| l == "two=idle"),
        "the pane never went working -> idle, so this proves nothing: {said:?}"
    );
    assert!(
        runner.started.borrow().is_empty(),
        "an ordinary shell chimed at the end of a command"
    );
}

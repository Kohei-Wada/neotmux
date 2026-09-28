//! The §4.11 fixture plugin set: real `.lua` files under `tests/fixtures`, loaded through
//! the ordinary runtime-layer resolution and the ordinary `require`, driven over a PTY.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod harness;
mod session;
use harness::{COLS, Neotmux, ROWS};

const PATIENCE: Duration = Duration::from_secs(10);

/// The fixture files live in their own layer; each test supplies a thin layer in front of
/// it holding only the `init.lua` that chooses what to require.
fn start(init: &str, shell: &str) -> (Neotmux, tempfile::TempDir) {
    let dir = harness::runtime(&[("init.lua", init)]);
    let fixtures = harness::fixture_layer();
    let nt = Neotmux::start_layered(&[dir.path(), &fixtures], shell);
    // The dir is handed back so it outlives the process that reads its init.lua.
    (nt, dir)
}

/// As `start`, but neotmux is launched from `cwd`: the tests that ask where a pane *begins*
/// need a beginning they chose.
fn start_in(init: &str, shell: &str, cwd: &Path) -> (Neotmux, tempfile::TempDir) {
    let dir = harness::runtime(&[("init.lua", init)]);
    let fixtures = harness::fixture_layer();
    let nt = Neotmux::start_layered_in(&[dir.path(), &fixtures], shell, cwd);
    (nt, dir)
}

/// The shipped layer behind a thin user layer, which is what an installed neotmux runs:
/// every shipped default loads unless the user's own `init.lua` turns it off.
fn start_shipped(init: &str, shell: &str) -> (Neotmux, tempfile::TempDir) {
    let dir = harness::runtime(&[("init.lua", init)]);
    let nt = Neotmux::start_layered(&[dir.path(), &shipped_layer()], shell);
    (nt, dir)
}

fn line_with<'a>(log: &'a str, marker: &str) -> &'a str {
    log.lines()
        .find(|line| line.contains(marker))
        .unwrap_or_else(|| panic!("no log line contains {marker:?}:\n{log}"))
}

/// Half a frame, with the frame at 16 ms. The median of a run of keystrokes on an idle
/// session measures about a millisecond, so this leaves the room a loaded runner needs
/// while still failing outright if a keystroke ever waits for the next frame boundary.
const ECHO_BUDGET: Duration = Duration::from_millis(6);
const ECHO_SAMPLES: usize = 40;

/// How long to leave the session quiet before the next sample. A fixed wait would alias
/// with the frame: every keystroke would land at the same point of it, and the run would
/// measure one phase of the loop rather than all of them.
fn quiet_before(sample: usize) -> Duration {
    Duration::from_millis(20 + (sample as u64 * 7) % 17)
}

/// Typing must not cost a frame. The loop has to wake on the pane's output rather than find
/// it on its next pass, and this measures the difference the way a hand feels it: a byte
/// into the terminal, the first byte of the redraw back out.
///
/// The median rather than the worst sample, because one descheduled moment on a busy runner
/// says nothing about the loop.
#[test]
fn a_keystroke_is_echoed_back_well_inside_a_frame() {
    let dir = harness::runtime(&[("init.lua", "")]);
    // A small surface, so what is measured is the loop noticing the output rather than the
    // cost of drawing a full screen under an unoptimised test build.
    let mut nt = Neotmux::start_sized(&[dir.path()], "/bin/cat", 10, 40);
    // The shell's own startup output must be over before the first sample is taken.
    nt.settle(Duration::from_millis(500));

    let mut samples: Vec<Duration> = (0..ECHO_SAMPLES)
        .filter_map(|i| nt.round_trip("x", quiet_before(i), PATIENCE))
        .collect();
    assert_eq!(
        samples.len(),
        ECHO_SAMPLES,
        "only {} of {ECHO_SAMPLES} keystrokes were ever echoed back",
        samples.len()
    );

    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    assert!(
        median <= ECHO_BUDGET,
        "a keystroke took a median {median:?} to come back, budget {ECHO_BUDGET:?}: {samples:?}"
    );
}

#[test]
fn clock_draws_a_status_line_sized_from_ctx_size_on_every_frame() {
    let (mut nt, _init) = start("require('plugin.clock')", "/bin/cat");
    // 80 columns and 23 rows: the terminal is 24 rows and the status line takes one.
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
    for _ in 0..3 {
        let screen = nt.screen_after(Duration::from_millis(300));
        assert!(
            screen.contains("CLOCK 80x23"),
            "the status line vanished, so some frame contributed nothing:\n{screen}"
        );
    }
}

#[test]
fn watcher_sees_the_bytes_the_harness_wrote_to_the_pty() {
    let (mut nt, _init) = start("require('plugin.watcher')", "cat");
    nt.send("WATCH-ME\r");
    let log = nt.wait_for_log("watcher saw WATCH-ME", PATIENCE);
    let line = line_with(&log, "watcher saw WATCH-ME");
    assert!(line.contains("plugin/watcher"), "{line}");
    assert!(line.contains("on pane 1"), "{line}");
}

#[test]
fn binder_consumes_one_key_and_lets_the_other_reach_the_pty() {
    let (mut nt, _init) = start("require('plugin.binder')", "cat");
    nt.send("\x18"); // C-x, consumed by the fixture
    nt.send("\x19"); // C-y, refused by the fixture
    nt.send("\r");
    let screen = nt.screen_after(Duration::from_secs(2));
    assert!(
        !screen.contains("^X"),
        "the consumed key reached the shell:\n{screen}"
    );
    assert!(
        screen.contains("^Y"),
        "the fallthrough key never reached the shell:\n{screen}"
    );
}

#[test]
fn conflicting_is_refused_by_name_and_binder_keeps_the_key() {
    let (mut nt, _init) = start(
        "require('plugin.binder') require('plugin.conflicting')",
        "cat",
    );
    let log = nt.wait_for_log("conflicting refused:", PATIENCE);

    let refusal = line_with(&log, "conflicting refused:");
    assert!(
        refusal.contains("already bound by plugin/binder"),
        "the refusal should name the holder: {refusal}"
    );

    let conflict = line_with(&log, "keymap conflict on 'C-x'");
    assert!(
        conflict.contains("plugin/binder") && conflict.contains("plugin/conflicting"),
        "the host's conflict line should name both sources: {conflict}"
    );

    nt.send("\x18");
    nt.send("\r");
    let screen = nt.screen_after(Duration::from_secs(2));
    assert!(
        !screen.contains("^X"),
        "binder's binding stopped working after the refused registration:\n{screen}"
    );
}

#[test]
fn configured_reads_the_value_init_lua_mutated_on_the_config_holder() {
    let (nt, _init) = start(
        "require('neotmux.configured').greeting = 'mutated-greeting'\nrequire('plugin.configured')",
        "/bin/cat",
    );
    let log = nt.wait_for_log("configured greeting=", PATIENCE);
    let line = line_with(&log, "configured greeting=");
    assert!(line.contains("greeting=mutated-greeting"), "{line}");
    assert!(line.contains("plugin/configured"), "{line}");
}

#[test]
fn a_named_client_is_told_its_name_by_the_api() {
    // One unnamed client: `neotmux.session_name()` must answer `default`; the named case
    // is the one the sessions tests cover.
    let (nt, _init) = start("require('plugin.session_name')", "/bin/cat");
    nt.wait_for_log("session_name default", PATIENCE);
}

#[test]
fn remover_removes_twice_without_raising_and_its_callback_never_runs() {
    let (mut nt, _init) = start(
        "require('plugin.clock') require('plugin.remover')",
        "/bin/cat",
    );
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
    // Long enough for a removed callback to have run, if it were going to: nothing to wait for.
    nt.settle(Duration::from_secs(1));

    let log = nt.log_text();
    let line = line_with(&log, "remover removed twice");
    assert!(line.contains("first=true second=true"), "{line}");
    assert!(
        !log.contains("remover tick ran"),
        "a removed callback still ran:\n{log}"
    );
}

#[test]
fn the_pane_ops_apply_after_the_tick_and_in_the_order_they_were_pushed() {
    let (nt, _init) = start("require('plugin.panes')", "/bin/cat");
    let log = nt.wait_for_log("panes applied list=", PATIENCE);

    let listed = line_with(&log, "panes list=");
    assert!(listed.contains("list=1 first=1 current=1"), "{listed}");

    let bad = line_with(&log, "panes bad direction");
    assert!(bad.contains("ok=false"), "{bad}");
    assert!(bad.contains("unknown split direction"), "{bad}");

    assert!(
        line_with(&log, "panes list after split=").contains("after split=1"),
        "split mutated the pane list synchronously, inside the callback"
    );

    // The split focuses the pane it made, so a focus back to %1 surviving means the two ops
    // drained in the order they were pushed (§4.6) rather than in the order they ran.
    let applied = line_with(&log, "panes applied list=");
    assert!(applied.contains("list=2 current=1"), "{applied}");
}

#[test]
fn a_callback_registered_during_another_plugins_dispatch_is_owned_by_its_own_chunk() {
    let (nt, _init) = start("require('plugin.dispatcher')", "/bin/cat");
    let log = nt.wait_for_log("late-callback-boom", PATIENCE);
    let line = line_with(&log, "late-callback-boom");
    assert!(
        line.contains("plugin/late"),
        "a callback registered from inside plugin/dispatcher's tick should still be owned \
         by the chunk it was written in: {line}"
    );
    assert!(
        !line.contains("plugin/dispatcher"),
        "the dispatching plugin was blamed for a callback it did not write: {line}"
    );
}

/// The load-convention guarantee from a plugin author's seat: one entry in the init chain
/// that cannot be resolved does not take the rest of the config with it.
#[test]
fn a_missing_plugin_in_the_init_chain_does_not_stop_the_rest() {
    let (mut nt, _init) = start(
        "require('plugin.not_installed') require('plugin.clock')",
        "/bin/cat",
    );
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
    let log = nt.log_text();
    let line = line_with(&log, "plugin.not_installed");
    assert!(line.contains("not found"), "{line}");
}

#[test]
fn painter_draws_over_the_pane() {
    let (mut nt, _init) = start("require('plugin.painter')", "/bin/cat");
    nt.wait_for_screen("OVERLAY", PATIENCE);
}

#[test]
fn nothing_is_written_while_nothing_happens() {
    let dir = harness::runtime(&[("init.lua", "")]);
    let mut nt = Neotmux::start(&dir);
    // Startup's own frames have no last one to wait for; they are over by the time this is.
    nt.settle(Duration::from_secs(1));
    assert_eq!(nt.bytes_in(Duration::from_secs(1)), 0);
}

/// The prefix plugin §6 calls for: it binds nothing, so both the prefix key and the command
/// key after it reach it only through `on_key`, and both are swallowed before the PTY.
#[test]
fn prefix_swallows_its_key_and_the_next_one_while_an_unrelated_key_gets_through() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02"); // C-b, the prefix
    nt.send("z"); // the command key, bound by nobody
    nt.send("Q"); // unrelated, no prefix pending
    nt.send("\r");

    let screen = nt.screen_after(Duration::from_secs(2));
    // The panes only: the bar's row carries a working directory, and what is in one of those
    // is not this test's business.
    let panes = pane_area(&screen);
    assert!(
        !panes.contains("^A"),
        "the prefix key reached the shell:\n{screen}"
    );
    assert!(
        !panes.contains('z'),
        "the command key after the prefix reached the shell:\n{screen}"
    );
    assert!(
        panes.contains('Q'),
        "the unrelated key never reached the shell:\n{screen}"
    );
}

/// Task 2: one PTY per pane. Both children are alive and each one's output lands in its own
/// half of the screen, never in the other's.
#[test]
fn each_pane_has_its_own_shell_and_its_own_grid() {
    let (mut nt, _init) = start("require('plugin.splitter')", "cat");
    nt.send("\x02"); // C-b: split vertically, focusing the new pane
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    nt.send("RIGHT");
    nt.wait_for_screen("RIGHT", PATIENCE);
    nt.send("\x0f"); // C-o: focus the other pane
    nt.wait_for_log("splitter focus_changed 1", PATIENCE);
    nt.send("LEFT");

    nt.wait_for_screen("LEFT", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(500));
    let row = screen.lines().next().unwrap_or_default().to_string();

    let left = row
        .find("LEFT")
        .unwrap_or_else(|| panic!("the left pane never echoed its own input:\n{screen}"));
    let right = row
        .find("RIGHT")
        .unwrap_or_else(|| panic!("the right pane never echoed its own input:\n{screen}"));
    assert!(
        left < 40,
        "pane %1's output is not in its own half of the surface:\n{screen}"
    );
    assert!(
        right >= 40,
        "pane %2's output landed in pane %1's grid:\n{screen}"
    );
}

/// Task 2: a key no plugin consumed reaches the focused pane's PTY and no other.
#[test]
fn an_unconsumed_key_reaches_only_the_focused_panes_pty() {
    let (mut nt, _init) = start("require('plugin.splitter')", "cat");
    nt.send("\x02");
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    nt.send("MARK");
    nt.wait_for_screen("MARK", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(500));
    assert_eq!(
        screen.matches("MARK").count(),
        1,
        "the key reached more than the focused pane:\n{screen}"
    );
}

/// Task 3 step 5: a split the surface cannot hold is reported against the plugin that asked
/// for it, and nothing changes.
#[test]
fn a_refused_split_is_reported_against_the_calling_source() {
    let dir = harness::runtime(&[("init.lua", "require('plugin.splitter')")]);
    let fixtures = harness::fixture_layer();
    let mut nt = Neotmux::start_sized(&[dir.path(), &fixtures], "cat", 24, 2);

    nt.send("\x02");
    let log = nt.wait_for_log("pane.split", PATIENCE);
    let refusal = line_with(&log, "pane.split");
    assert!(refusal.contains("plugin/splitter"), "{refusal}");
    assert!(refusal.contains("too small"), "{refusal}");

    nt.send("\x07"); // C-g: report the pane list
    let log = nt.wait_for_log("splitter panes=", PATIENCE);
    let listed = line_with(&log, "splitter panes=");
    assert!(listed.contains("panes=1 current=1"), "{listed}");
}

/// Task 3 step 7: focusing a pane that does not exist is reported, and the session lives on.
#[test]
fn focusing_an_unknown_pane_is_reported_and_does_not_stop_the_session() {
    let (mut nt, _init) = start("require('plugin.splitter')", "cat");
    nt.send("\x07"); // C-g: report, so the next key is known to arrive after init
    nt.wait_for_log("splitter panes=", PATIENCE);

    nt.send("\x15"); // C-u: focus a pane id nobody owns
    let log = nt.wait_for_log("pane.focus", PATIENCE);
    let refusal = line_with(&log, "pane.focus");
    assert!(refusal.contains("plugin/splitter"), "{refusal}");
    assert!(refusal.contains("no such pane"), "{refusal}");

    nt.send("\x07");
    nt.wait_until("the second report never came", PATIENCE, |nt| {
        nt.log_text().matches("splitter panes=").count() >= 2
    });
    let log = nt.log_text();
    assert!(
        line_with(&log, "splitter panes=").contains("panes=1 current=1"),
        "{log}"
    );
}

/// Task 3 step 6: one pane's shell exiting closes that pane only — the survivor keeps
/// running, takes the space, and takes the focus.
#[test]
fn a_pane_whose_shell_exits_is_closed_and_its_sibling_takes_over() {
    let (mut nt, _init) = start("require('plugin.splitter')", "cat");
    nt.send("\x02");
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    nt.send("\x04"); // C-d: EOF for the focused pane's cat
    nt.wait_for_log("splitter pane_closed 2", PATIENCE);

    nt.send("\x07");
    let log = nt.wait_for_log("splitter panes=", PATIENCE);
    let listed = line_with(&log, "splitter panes=");
    assert!(listed.contains("panes=1 current=1"), "{listed}");

    nt.send("SURVIVOR");
    nt.wait_for_screen("SURVIVOR", PATIENCE);
}
/// §5.5's first exit criterion in the running program: the offender loses its tick, the
/// well-behaved neighbour keeps drawing, and the child is still there to talk to.
fn spinner_is_dropped_while_the_rest_of_the_program_carries_on(module: &str, echo: &str) {
    let (mut nt, _init) = start(
        &format!(
            "neotmux.opt.tick_budget_ms = 5 neotmux.opt.tick_overrun_limit = 3 \
             require('plugin.clock') require('{module}')"
        ),
        "cat",
    );

    let log = nt.wait_for_log("no longer ticking", PATIENCE);
    let dropped: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("no longer ticking"))
        .collect();
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(
        dropped[0].contains(module.replace('.', "/").as_str()),
        "{}",
        dropped[0]
    );
    assert!(
        !log.contains("ran past its loop"),
        "the tick was not cut off:\n{log}"
    );

    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
    nt.send(echo);
    nt.send("\r");
    let screen = nt.screen_after(Duration::from_secs(2));
    assert!(
        screen.contains(echo),
        "the shell stopped answering after the overrun:\n{screen}"
    );
    // Waited for, not asserted on the frame above. The notification naming the offender is
    // drawn along the bottom edge of the host surface (`notices.rs`) and the clock fixture
    // reserves that same row, so for the notice's four seconds the band is covered by it —
    // by design, since a source's own draw buffer cannot carry a notification about the
    // source. What this is asking is that the well-behaved neighbour is still drawing, and
    // that is the band coming back when the notice goes, not the band being visible at
    // whichever instant the capture landed on. Asserting the instant passed only by luck:
    // the wait above blocks for the notice's whole lifetime when the notice is already up,
    // and the once in six that it caught the frame before the notice was drawn, the capture
    // two seconds later was still inside those four and this failed (issue #6).
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
}

#[test]
fn a_spinner_loses_its_tick_and_the_terminal_stays_usable() {
    spinner_is_dropped_while_the_rest_of_the_program_carries_on("plugin.spinner", "STILL-HERE");
}

#[test]
fn a_coroutine_wrapped_spinner_loses_its_tick_too() {
    spinner_is_dropped_while_the_rest_of_the_program_carries_on(
        "plugin.coroutine_spinner",
        "STILL-HERE-TOO",
    );
}

/// Task 4: the prefix does what a prefix is for. `C-b v` splits vertically and `C-b o`
/// walks to the next pane, both expressed with nothing but `neotmux.pane`.
#[test]
fn the_prefix_splits_vertically_and_walks_to_the_next_pane() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v"); // C-b v
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);

    nt.send("RIGHT");
    nt.wait_for_screen("RIGHT", PATIENCE);
    nt.send("\x02o"); // C-b o
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    nt.send("LEFT");

    nt.wait_for_screen("LEFT", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(500));
    let row = pane_rows(&screen).next().unwrap_or_default().to_string();

    let left = row
        .find("LEFT")
        .unwrap_or_else(|| panic!("the pane the prefix walked back to never echoed:\n{screen}"));
    let right = row
        .find("RIGHT")
        .unwrap_or_else(|| panic!("`C-b v` never produced a second pane:\n{screen}"));
    assert!(left < 40 && right >= 40, "{screen}");
    let panes = pane_area(&screen);
    assert!(
        !panes.contains('v') && !panes.contains("^B"),
        "a prefix keystroke reached a shell:\n{screen}"
    );
}

/// Task 4: `C-b -` splits the other way — the two panes are stacked, not side by side.
#[test]
fn the_prefix_splits_horizontally() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02-"); // C-b -
    nt.wait_for_cell(DIVIDER_ROW, 0, HORIZONTAL_DIVIDER, PATIENCE);

    nt.send("BELOW");
    nt.wait_for_screen("BELOW", PATIENCE);
    nt.send("\x02o");
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    nt.send("ABOVE");

    nt.wait_for_screen("ABOVE", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(500));
    let line_of = |needle: &str| {
        screen
            .lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} never reached a pane:\n{screen}"))
    };
    assert!(
        line_of("ABOVE") < line_of("BELOW"),
        "the panes are not stacked, so the split was not horizontal:\n{screen}"
    );
    // The bar's own row is left out: a working directory with a dash in it is not a leak.
    assert!(
        !pane_area(&screen).contains('-'),
        "the command key reached the shell:\n{screen}"
    );
}

/// The shipped surface is 80 columns and, with the status bar's row taken off the top, rows
/// 1..23. A gutter of one goes between the panes, so a vertical split leaves the left pane
/// columns 0..39, the divider column 40 and the right pane the rest; a horizontal one leaves
/// the top pane rows 1..11, the divider row 12 and the bottom pane the rest.
/// The cap `plugin.status` opens every block of the bar with, and what the bar says about a
/// window whose focused pane is running `cat`: the window's position, the focused pane's
/// position after it, then that command.
const CAP_LEFT: &str = "\u{e0b6}";
const FIRST_WINDOW: &str = "1.1  cat";
const SECOND_WINDOW: &str = "2.1  cat";
const THIRD_WINDOW: &str = "3.1  cat";
/// The same once a split has landed and left the new pane focused. The bar now says both of
/// the things a test has to wait for after a split, so it is what they wait on.
const FIRST_PANE: &str = "1.1  cat";
const SECOND_PANE: &str = "1.2  cat";
const THIRD_PANE: &str = "1.3  cat";

const PANE_TOP: u16 = 1;
const PANE_ROWS: usize = 23;
const PANE_COLS: usize = 80;
const DIVIDER_COL: u16 = 40;
const DIVIDER_ROW: u16 = 12;
const VERTICAL_DIVIDER: &str = "\u{2502}";
const HORIZONTAL_DIVIDER: &str = "\u{2500}";

/// The screen below the bar's row: what the shells were given, and the only place a
/// keystroke that leaked past the prefix could show up.
fn pane_rows(screen: &str) -> impl Iterator<Item = &str> {
    screen.lines().skip(PANE_TOP as usize)
}

/// The pane once it has stopped changing: two consecutive reads that agree. A fixed sleep
/// cannot express this — under a loaded runner the frame after it is still moving, which is
/// how the copy-mode comparison below used to flake.
fn settled_pane_area(nt: &mut Neotmux, patience: Duration) -> String {
    let deadline = Instant::now() + patience;
    let mut last = pane_area(&nt.screen_after(Duration::from_millis(100)));
    loop {
        let next = pane_area(&nt.screen_after(Duration::from_millis(100)));
        if next == last {
            return next;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never stopped changing:\n{next}"
        );
        last = next;
    }
}

fn pane_area(screen: &str) -> String {
    pane_rows(screen).collect::<Vec<_>>().join("\n")
}

/// The UI a split had no way to show: with the panes tiling the surface exactly, nothing on
/// screen said where one ended. `C-b v` now rules the reserved column between them, and a
/// shell is still live on each side of it.
#[test]
fn a_vertical_split_draws_a_divider_column_between_the_panes() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v"); // C-b v
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);

    nt.send("RIGHT");
    nt.wait_for_screen("RIGHT", PATIENCE);
    nt.send("\x02o"); // C-b o
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    nt.send("LEFT");
    nt.wait_for_screen("LEFT", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    let ruled = pane_rows(&screen)
        .take(PANE_ROWS)
        .filter(|row| row.chars().nth(DIVIDER_COL as usize) == VERTICAL_DIVIDER.chars().next())
        .count();
    assert_eq!(
        ruled, PANE_ROWS,
        "no divider column down the boundary between the panes:\n{screen}"
    );

    let top = pane_rows(&screen).next().unwrap_or_default();
    let left = top
        .find("LEFT")
        .unwrap_or_else(|| panic!("the left pane never echoed:\n{screen}"));
    let right = top
        .find("RIGHT")
        .unwrap_or_else(|| panic!("the right pane never echoed:\n{screen}"));
    assert!(
        left < DIVIDER_COL as usize && right > DIVIDER_COL as usize,
        "the divider is not between the two shells:\n{screen}"
    );
}

/// The same, the other way up: `C-b -` rules the boundary row, with a shell above and below.
#[test]
fn a_horizontal_split_draws_a_divider_row_between_the_panes() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02-"); // C-b -
    nt.wait_for_cell(DIVIDER_ROW, 0, HORIZONTAL_DIVIDER, PATIENCE);

    nt.send("BELOW");
    nt.wait_for_screen("BELOW", PATIENCE);
    nt.send("\x02o"); // C-b o
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    nt.send("ABOVE");
    nt.wait_for_screen("ABOVE", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    let rows: Vec<&str> = screen.lines().collect();
    let rule = rows.get(DIVIDER_ROW as usize).copied().unwrap_or_default();
    assert_eq!(
        rule.matches(HORIZONTAL_DIVIDER).count(),
        PANE_COLS,
        "no divider row across the boundary between the panes:\n{screen}"
    );

    let line_of = |needle: &str| {
        rows.iter()
            .position(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("{needle} never reached a pane:\n{screen}"))
    };
    assert!(
        line_of("ABOVE") < DIVIDER_ROW as usize && line_of("BELOW") > DIVIDER_ROW as usize,
        "the divider is not between the two shells:\n{screen}"
    );
}

/// The junctions, one per set of arms that can meet in a cell. A tee is named for the arm
/// that is on its own, which is the way its stem points.
const CROSS: &str = "\u{253c}";
const TEE_RIGHT: &str = "\u{251c}";
const TEE_LEFT: &str = "\u{2524}";
const TEE_DOWN: &str = "\u{252c}";

/// The crossing cell of a 2x2 grid is in the vertical gutter and in the horizontal one, so
/// it is inside neither pane's extent and no pane draws it. It used to be left blank: a
/// cross with a hole in the middle.
#[test]
fn a_two_by_two_grid_meets_in_a_cross() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v"); // C-b v: a left pane and a right one
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);
    nt.send("\x02-"); // C-b -: the right one, split
    nt.wait_for_cell(DIVIDER_ROW, DIVIDER_COL + 1, HORIZONTAL_DIVIDER, PATIENCE);
    nt.send("\x02h"); // C-b h: back to the left pane
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    nt.send("\x02-"); // C-b -: and split that one too
    nt.wait_for_cell(DIVIDER_ROW, 0, HORIZONTAL_DIVIDER, PATIENCE);

    nt.wait_for_cell(DIVIDER_ROW, DIVIDER_COL, CROSS, PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(200));
    assert_eq!(
        (
            nt.cell(DIVIDER_ROW - 1, DIVIDER_COL),
            nt.cell(DIVIDER_ROW + 1, DIVIDER_COL),
            nt.cell(DIVIDER_ROW, DIVIDER_COL - 1),
            nt.cell(DIVIDER_ROW, DIVIDER_COL + 1),
        ),
        (
            VERTICAL_DIVIDER.to_string(),
            VERTICAL_DIVIDER.to_string(),
            HORIZONTAL_DIVIDER.to_string(),
            HORIZONTAL_DIVIDER.to_string(),
        ),
        "the cross does not have four arms to be a cross of:\n{screen}"
    );
}

/// A full-height pane on the left with the right-hand column split: the right column's
/// divider runs into the vertical one and stops. Three arms, not four, so `┼` would draw a
/// left arm through the middle of the left pane's output.
#[test]
fn a_divider_running_into_another_from_the_right_is_a_tee() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v"); // C-b v: the new pane on the right is the focused one
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);
    nt.send("\x02-"); // C-b -: so this splits the right-hand column
    nt.wait_for_cell(DIVIDER_ROW, DIVIDER_COL + 1, HORIZONTAL_DIVIDER, PATIENCE);

    nt.wait_for_cell(DIVIDER_ROW, DIVIDER_COL, TEE_RIGHT, PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(200));
    assert_ne!(
        nt.cell(DIVIDER_ROW, DIVIDER_COL - 1),
        HORIZONTAL_DIVIDER.to_string(),
        "an arm was drawn into the left pane, which is not divided there:\n{screen}"
    );
}

/// The mirror image: the column that is split is the left one, so the arm points the other
/// way.
#[test]
fn a_divider_running_into_another_from_the_left_is_the_mirror_tee() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v");
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);
    nt.send("\x02h"); // C-b h: onto the left-hand column
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    nt.send("\x02-");
    nt.wait_for_cell(DIVIDER_ROW, 0, HORIZONTAL_DIVIDER, PATIENCE);

    nt.wait_for_cell(DIVIDER_ROW, DIVIDER_COL, TEE_LEFT, PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(200));
    assert_ne!(
        nt.cell(DIVIDER_ROW, DIVIDER_COL + 1),
        HORIZONTAL_DIVIDER.to_string(),
        "an arm was drawn into the right pane, which is not divided there:\n{screen}"
    );
}

/// A wide pane above two side by side: the vertical divider starts at the horizontal one and
/// goes down only.
#[test]
fn a_divider_starting_at_another_and_going_down_is_a_tee_down() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02-"); // C-b -: the new pane along the bottom is the focused one
    nt.wait_for_cell(DIVIDER_ROW, 0, HORIZONTAL_DIVIDER, PATIENCE);
    nt.send("\x02v"); // C-b v: so this splits the bottom half in two
    nt.wait_for_cell(DIVIDER_ROW + 1, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);

    nt.wait_for_cell(DIVIDER_ROW, DIVIDER_COL, TEE_DOWN, PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(200));
    assert_ne!(
        nt.cell(DIVIDER_ROW - 1, DIVIDER_COL),
        VERTICAL_DIVIDER.to_string(),
        "an arm was drawn up into the top pane, which is not divided:\n{screen}"
    );
}

/// Nothing crosses a lone divider, so nothing along it is a junction. Two arms in line are a
/// line carrying on.
#[test]
fn a_lone_divider_has_no_junction_anywhere_along_it() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v");
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    let ruled = pane_rows(&screen)
        .take(PANE_ROWS)
        .filter(|row| row.chars().nth(DIVIDER_COL as usize) == VERTICAL_DIVIDER.chars().next())
        .count();
    assert_eq!(
        ruled, PANE_ROWS,
        "the divider is not whole from top to bottom:\n{screen}"
    );
    for junction in [CROSS, TEE_RIGHT, TEE_LEFT, TEE_DOWN] {
        assert!(
            !screen.contains(junction),
            "{junction:?} was drawn where nothing crosses:\n{screen}"
        );
    }
}

/// `C-b r` and five presses of the left arrow: the divider is five columns further left
/// than it was, which is the only place a resize can be seen. The mode is the plugin's own
/// state, so the keys in it never reach a shell, and the key that leaves it gives them back.
#[test]
fn resize_mode_moves_the_divider_five_columns_and_gives_the_keys_back() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v"); // C-b v: the new pane on the right is the focused one
    nt.wait_for_status(SECOND_PANE, PATIENCE);
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);

    nt.send("\x02r"); // C-b r: into resize mode
    for _ in 0..5 {
        nt.send("h"); // one cell each, the way every mode neotmux ships moves
    }

    let moved = DIVIDER_COL - 5;
    nt.wait_for_cell(PANE_TOP, moved, VERTICAL_DIVIDER, PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(200));
    assert_eq!(
        nt.cell(PANE_TOP, moved),
        VERTICAL_DIVIDER.to_string(),
        "the divider did not move five columns left:\n{screen}"
    );
    assert_ne!(
        nt.cell(PANE_TOP, DIVIDER_COL),
        VERTICAL_DIVIDER.to_string(),
        "the divider is still where it was:\n{screen}"
    );
    // The bar's own row is left out: it names the command the pane is running, and a session
    // started on `cat` would fail this on the `r` of its own report.
    let panes = pane_area(&screen);
    assert!(
        !panes.contains("^B") && !panes.contains('r'),
        "a key of the mode reached a shell:\n{screen}"
    );

    nt.send("\r"); // anything that is not an adjustment leaves the mode
    nt.send("TYPED");
    nt.wait_for_screen("TYPED", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(300));
    let at = pane_rows(&screen)
        .next()
        .unwrap_or_default()
        .find("TYPED")
        .unwrap_or_else(|| panic!("the focused pane stopped taking keys:\n{screen}"));
    assert!(
        at > usize::from(moved),
        "what was typed after the mode landed left of the divider:\n{screen}"
    );
}

/// `h/j/k/l` over a nested layout: a tall pane on the left and two stacked on the right.
/// From the bottom-right pane `k` is the one above it and `h` is the tall one beside it —
/// asserted by typing into whatever ends up focused and seeing where the text lands.
#[test]
fn directional_focus_lands_on_the_pane_a_person_would_point_at() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v"); // C-b v: pane 2, the right-hand column, is focused
    nt.wait_for_status(SECOND_PANE, PATIENCE);
    nt.send("\x02-"); // C-b -: pane 3, the bottom of that column, is focused
    nt.wait_for_status(THIRD_PANE, PATIENCE);

    nt.send("CCC");
    nt.wait_for_screen("CCC", PATIENCE);
    nt.send("\x02k"); // C-b k: up, to the top of the right-hand column
    nt.wait_for_status(SECOND_PANE, PATIENCE);
    nt.send("BBB");
    nt.wait_for_screen("BBB", PATIENCE);
    nt.send("\x02h"); // C-b h: left, to the tall pane beside it
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    nt.send("AAA");
    nt.wait_for_screen("AAA", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    let at = |needle: &str| {
        screen
            .lines()
            .enumerate()
            .find_map(|(row, line)| line.find(needle).map(|col| (row, col)))
            .unwrap_or_else(|| panic!("{needle} never reached a pane:\n{screen}"))
    };
    let (a_row, a_col) = at("AAA");
    let (b_row, b_col) = at("BBB");
    let (c_row, c_col) = at("CCC");
    assert!(
        a_col < usize::from(DIVIDER_COL),
        "`h` did not land in the left-hand pane:\n{screen}"
    );
    assert!(
        b_col > usize::from(DIVIDER_COL) && b_row < usize::from(DIVIDER_ROW),
        "`k` did not land in the pane above:\n{screen}"
    );
    assert!(
        c_col > usize::from(DIVIDER_COL) && c_row > usize::from(DIVIDER_ROW),
        "the split did not leave the bottom-right pane focused:\n{screen}"
    );
    assert!(
        a_row < usize::from(DIVIDER_ROW),
        "the left-hand pane is the whole column, so its shell starts at the top:\n{screen}"
    );
}

/// Task 4 step 5: the prefix is a plugin, not a feature. Switched off from a user config —
/// the same one line that switches the status line off — `C-b` is an ordinary keystroke and
/// the multiplexer is otherwise unchanged. That the defaults table works for a second entry
/// is the point: it is a convention, not a special case for the status line.
#[test]
fn a_user_config_can_switch_off_the_shipped_prefix() {
    let (mut nt, _init) = start_shipped("require('neotmux.defaults').prefix = false", "cat");
    nt.send("\x02"); // C-b, now bound by nobody
    nt.send("v");
    nt.send("PLAIN\r");

    nt.wait_for_screen("PLAIN", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(500));
    assert!(
        screen.contains("^B"),
        "C-b was swallowed with no prefix plugin loaded:\n{screen}"
    );
    assert!(
        screen.contains('v'),
        "the command key was swallowed with no prefix plugin loaded:\n{screen}"
    );
    assert!(
        screen.contains("PLAIN"),
        "the one pane stopped echoing:\n{screen}"
    );
}

/// The runtime layer neotmux ships: the one an install puts on the default search path,
/// read here straight out of the source tree.
fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

/// Task 5: the default status bar is a shipped plugin, so it is there with no user config at
/// all — in the row it reserved, naming the window it is showing and what the focused pane
/// is running.
#[test]
fn the_shipped_status_line_shows_with_no_user_config() {
    let init = harness::runtime(&[("init.lua", "require('plugin.splitter')")]);
    let fixtures = harness::fixture_layer();
    let shipped = shipped_layer();
    let mut nt = Neotmux::start_layered(&[init.path(), &fixtures, &shipped], "cat");

    let status = nt.wait_for_status("cat", PATIENCE);
    assert!(status.contains("1"), "the window is not listed: {status:?}");
    nt.send("\x02"); // C-b: split, focusing the new pane
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    nt.send("\x0f"); // C-o: focus the other one
    nt.wait_for_log("splitter focus_changed 1", PATIENCE);
    nt.wait_for_status("cat", PATIENCE);
    assert_eq!(
        nt.cell(0, 0),
        CAP_LEFT,
        "the status bar is not in the row it reserved:\n{}",
        nt.screen_after(Duration::ZERO)
    );
}

/// The requirement the user stated first, on the screen a user sees: the bar does not merely
/// change when the focus moves, it says something different about the pane the focus is on.
#[test]
fn the_bar_says_something_different_about_each_focused_pane() {
    let (mut nt, _init) = start_shipped("", "bash");
    nt.wait_for_status("bash", PATIENCE);

    nt.send("\x02v"); // C-b v: a second pane, focused
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);
    nt.send("sleep 30\n");
    let running = nt.wait_for_status("sleep", PATIENCE);

    nt.send("\x02o"); // C-b o: back to the pane sitting at its prompt
    let prompt = nt.wait_for_status("bash", PATIENCE);

    assert_ne!(
        running, prompt,
        "the bar says the same thing about both panes"
    );
    assert!(
        !prompt.contains("sleep"),
        "the bar is still describing the pane the focus left: {prompt:?}"
    );
}

/// Nothing else wakes the frame. A command starting, that command ending and a `cd` each move
/// the bar on their own — the second of those with no keystroke at all in between.
#[test]
fn the_bar_follows_a_command_and_a_cd_with_no_keystroke() {
    let (mut nt, _init) = start_shipped("", "bash");
    nt.wait_for_status("bash", PATIENCE);

    nt.send("sleep 2\n");
    nt.wait_for_status("sleep", PATIENCE);
    // Nothing is sent from here: the sleep finishing is the only thing that moves the bar.
    nt.wait_for_status("bash", PATIENCE);

    let target = std::fs::canonicalize("/tmp").expect("canonicalise /tmp");
    nt.send(&format!("cd {}\n", target.display()));
    let status = nt.wait_for_status(&target.display().to_string(), PATIENCE);
    assert!(
        status.contains("bash"),
        "the bar lost the command when the directory changed: {status:?}"
    );
}

/// A `PaneId` is allocated once and never reused, so after two splits and two closes the
/// living pane's id is 3. A bar that showed the id counted up for ever; what it shows is the
/// pane's place in `pane.list()`, which closes the gap the closed pane left.
#[test]
fn closing_a_pane_leaves_the_bars_pane_number_where_it_was() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);

    for _ in 0..3 {
        nt.send("\x02v"); // C-b v
        // The second pane is always the second, however many have been and gone.
        nt.wait_for_status(SECOND_PANE, PATIENCE);
        nt.send("\x04"); // C-d: EOF to `cat`, so the pane's shell ends and the pane goes
        nt.wait_for_status(FIRST_WINDOW, PATIENCE);
    }

    let status = nt.status();
    assert!(
        !status.contains("1.4") && !status.contains("1.5"),
        "the bar is showing pane ids rather than positions: {status:?}"
    );
}

/// The same for windows. `window.list()` is in creation order and a `WindowId` is not reused
/// either, so the bar counts positions there too: opening and closing a second window keeps
/// saying 2, never 3.
#[test]
fn closing_a_window_leaves_the_bars_window_number_where_it_was() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);

    for _ in 0..3 {
        nt.send("\x02c"); // C-b c: a window, shown
        nt.wait_for_status(SECOND_WINDOW, PATIENCE);
        nt.send("\x04"); // C-d: its only shell ends, so the window goes with it
        nt.wait_for_status(FIRST_WINDOW, PATIENCE);
    }

    let status = nt.status();
    assert!(
        !status.contains("3.1") && !status.contains("4.1"),
        "the bar is showing window ids rather than positions: {status:?}"
    );
}

/// The defect the gutter exists for, measured the way it was found: on the screen, over a
/// PTY. A vertical split of an 80-column terminal gives the left pane 40 columns, and the
/// border used to be drawn on the last of them — so the 40th character a shell printed was
/// replaced by it. With the host reserving the divider's column, all 40 are still there, the
/// divider is beside them, and the right pane's first column is its own too.
#[test]
fn a_border_takes_no_column_from_either_pane_it_divides() {
    const FILL: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcd";
    let init = harness::runtime(&[("init.lua", "require('plugin.splitter')")]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &harness::fixture_layer(), &shipped_layer()],
        "cat",
    );

    nt.wait_for_status("cat", PATIENCE);
    nt.send("\x02"); // C-b: splitter splits the width, so the new pane is the right one
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    nt.send("R"); // into the right pane's first column, which the divider must not take
    nt.send("\x0f"); // C-o: back to the left pane
    // The focus op is applied at the end of the frame the key was read in, so keys sent
    // before the event lands would still go to the pane on the right.
    nt.wait_for_log("splitter focus_changed 1", PATIENCE);
    nt.send(FILL);

    nt.wait_for_screen(FILL, PATIENCE);
    assert_eq!(
        nt.cell(PANE_TOP, 39),
        "d",
        "the border ate the last column of the left pane:\n{}",
        nt.screen_after(Duration::ZERO)
    );
    assert_eq!(
        nt.cell(PANE_TOP, DIVIDER_COL),
        VERTICAL_DIVIDER,
        "the divider is not in the reserved column:\n{}",
        nt.screen_after(Duration::ZERO)
    );
    assert_eq!(
        nt.cell(PANE_TOP, 41),
        "R",
        "the border ate the first column of the right pane:\n{}",
        nt.screen_after(Duration::ZERO)
    );
}

/// The other half of the bargain: the gutter is reserved because a plugin asked for it, so
/// with that plugin switched off the panes tile the surface again and pay nothing.
#[test]
fn switching_the_borders_off_gives_the_panes_their_cells_back() {
    let init = harness::runtime(&[(
        "init.lua",
        "require('neotmux.defaults').borders = false\nrequire('plugin.splitter')",
    )]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &harness::fixture_layer(), &shipped_layer()],
        "cat",
    );

    nt.wait_for_status("cat", PATIENCE);
    nt.send("\x02"); // C-b: split, focusing the new pane on the right
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    nt.send("R");
    nt.wait_for_screen("R", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    assert!(
        !screen.contains(VERTICAL_DIVIDER),
        "a divider survived the opt-out:\n{screen}"
    );
    assert_eq!(
        nt.cell(PANE_TOP, DIVIDER_COL),
        "R",
        "the right pane does not start where a tiling layout puts it:\n{screen}"
    );
}

/// A reservation is the whole of what the status row used to be: the panes lose exactly the
/// rows asked for, and get them back when the band goes. Asserted on screen, because the
/// fixture reports the surface it was handed — `CLOCK 80x23` is one row reserved, `80x22` is
/// two — and because that surface is the thing a shell is actually given.
#[test]
fn reserving_a_row_shrinks_the_panes_and_releasing_it_gives_it_back() {
    let (mut nt, _init) = start("require('plugin.clock') require('plugin.bander')", "cat");
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);

    nt.send("\x12"); // C-r: bander reserves a second bottom row
    nt.wait_for_screen("CLOCK 80x22", PATIENCE);

    nt.send("\x14"); // C-t: bander gives it back
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        !screen.contains("BAND"),
        "a released band was still drawn in:\n{screen}"
    );
}

/// Two sources on one edge, each with its own rows: the one that asked first keeps the
/// outermost row and the second sits inside it. With one plugin loaded this is invisible,
/// which is why it is asserted by column and row rather than by "it is on screen".
#[test]
fn two_sources_reserving_the_bottom_get_a_row_each() {
    let (mut nt, _init) = start("require('plugin.clock') require('plugin.bander')", "cat");
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
    nt.send("\x12"); // C-r
    nt.wait_for_screen("BAND", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(300));
    let rows: Vec<&str> = screen.lines().collect();
    assert!(
        rows[23].starts_with("CLOCK 80x22"),
        "the first to reserve lost the outermost row:\n{screen}"
    );
    assert!(
        rows[22].starts_with("BAND"),
        "the second band did not get a row of its own:\n{screen}"
    );
}

/// The other half of the bargain: a band is outside every pane's rect, so a shell that
/// floods its pane can neither reach it nor be drawn over by it.
#[test]
fn a_flooding_shell_cannot_reach_a_reserved_band() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.wait_for_status("cat", PATIENCE);
    for _ in 0..30 {
        nt.send("ZZZZZZZZ\r");
    }
    // Every line of the flood is the same, so none of them says the flood is over.
    nt.settle(Duration::from_millis(500));

    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        pane_area(&screen).contains('Z'),
        "the shell never filled its pane:\n{screen}"
    );
    let status = nt.status();
    assert!(
        !status.contains("ZZ"),
        "the shell wrote into the reserved band:\n{screen}"
    );
    assert!(
        status.starts_with(CAP_LEFT) && status.contains("cat"),
        "the band's content did not survive the flood:\n{screen}"
    );
}

/// A reservation the edge cannot hold is refused as a value, the caller is told why, and
/// nothing about the surface changes.
#[test]
fn a_reservation_the_edge_cannot_hold_is_refused_and_changes_nothing() {
    let (mut nt, _init) = start("require('plugin.clock') require('plugin.greedy')", "cat");
    let log = nt.wait_for_log("greedy refused", PATIENCE);
    let refusal = line_with(&log, "greedy refused");
    assert!(
        refusal.contains("plugin/greedy") && refusal.contains("will not fit"),
        "the refusal did not say who asked or why: {refusal}"
    );

    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        screen.contains("CLOCK 80x23"),
        "the refused reservation took a row from the panes anyway:\n{screen}"
    );
    assert!(
        !nt.log_text().contains("drew in a band it was refused"),
        "the refused source was handed a band:\n{}",
        nt.log_text()
    );
}

/// The epoch hazard, in the shape it took the first time. A frame's draw buffers are
/// committed against the content size the frame opened with, so a reservation appearing
/// mid-frame must not change that size until the frame is over. Get it wrong and the band
/// blanks for a frame, the host reads the blank as "gone", takes the row back and gives it
/// straight back: two recomputes and two SIGWINCHes for one reservation.
#[test]
fn reserving_a_band_costs_one_layout_recompute() {
    let init = harness::runtime(&[(
        "init.lua",
        "require('plugin.resizes') require('plugin.bander')",
    )]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &harness::fixture_layer(), &shipped_layer()],
        "cat",
    );
    nt.wait_for_status("cat", PATIENCE);
    // Startup's own resizes have to be over before counting, and none of them is the last.
    nt.settle(Duration::from_millis(500));
    let before = resizes(&nt.log_text()).len();

    nt.send("\x12"); // C-r: reserve a second bottom row
    nt.wait_for_screen("BAND", PATIENCE);
    // A second recompute, the bug, would come after the one the wait saw: it has no marker.
    nt.settle(Duration::from_millis(500));

    let log = nt.log_text();
    let after = resizes(&log);
    assert_eq!(
        after.len(),
        before + 1,
        "one reservation did not cost exactly one recompute. Unexpected: {:?}",
        &after[before.min(after.len())..]
    );
    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        nt.status().contains("cat"),
        "the shipped band did not survive another source reserving one:\n{screen}"
    );
}

/// §1's fourth exit criterion for the *shipped* defaults: a user who does not want one
/// turns it off from their own config, in one line, without touching an installed file.
/// Every layer's `init.lua` runs earliest-first and the shipped layer is last, so the user's
/// assignment is in place by the time the shipped `init.lua` reads it.
#[test]
fn a_user_config_can_switch_off_a_shipped_default() {
    let init = harness::runtime(&[("init.lua", "require('neotmux.defaults').status = false")]);
    let shipped = shipped_layer();
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped], "cat");

    nt.send("STILL-HERE\r");
    nt.wait_for_screen("STILL-HERE", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    // The top row is the shell's again, which is the row the bar would have taken.
    assert!(
        !screen.contains(CAP_LEFT) && nt.status().contains("STILL-HERE"),
        "the shipped status bar ignored the opt-out:\n{screen}"
    );
    assert!(
        nt.log_text().is_empty(),
        "switching a default off was reported as a failure:\n{}",
        nt.log_text()
    );
}

/// §1's first exit criterion, checked by deletion rather than by grep: with the shipped
/// `lua/plugin/` gone there is no status line, no error about the missing file, and the
/// multiplexer still runs a shell and exits cleanly.
#[test]
fn deleting_the_shipped_plugins_leaves_a_working_multiplexer() {
    let stripped = tempfile::tempdir().expect("stripped layer");
    copy_tree(&shipped_layer(), stripped.path());
    std::fs::remove_dir_all(stripped.path().join("lua/plugin")).expect("remove the plugin dir");

    let mut nt = Neotmux::start_layered(&[stripped.path()], "cat");
    nt.send("\x02"); // C-b: an ordinary keystroke with no prefix plugin left to swallow it
    nt.send("STILL-HERE\r");
    nt.wait_for_screen("STILL-HERE", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    assert!(
        !screen.contains(CAP_LEFT) && nt.status().contains("STILL-HERE"),
        "a status bar survived the deletion:\n{screen}"
    );
    assert!(
        screen.contains("^B"),
        "a prefix survived the deletion of the file it lives in:\n{screen}"
    );
    let log = nt.log_text();
    assert!(
        log.is_empty(),
        "deleting the shipped plugins was reported as a failure:\n{log}"
    );

    nt.send("\x04"); // EOF: the last pane's shell exits, so neotmux does
    assert!(nt.exited_cleanly(PATIENCE), "neotmux did not exit cleanly");
}

/// Builds the install layout in a temp prefix: the Makefile puts the shipped runtime layer
/// in place, and the already-built test binary stands in for the copy `cargo install` makes.
fn install_into(prefix: &Path) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let status = std::process::Command::new("make")
        .current_dir(&root)
        .arg("install-runtime")
        .arg(format!("PREFIX={}", prefix.display()))
        .status()
        .expect("run make install-runtime");
    assert!(status.success(), "make install-runtime failed");

    std::fs::create_dir_all(prefix.join("bin")).expect("create bin");
    std::fs::copy(env!("CARGO_BIN_EXE_neotmux"), prefix.join("bin/neotmux")).expect("copy binary");
}

/// The layer an install actually produces, not the one in the source tree. Every other test
/// here points `NEOTMUX_RUNTIME` at `runtime/`, so they exercise the layer's content and
/// never its installed layout — which is how an install that copied `init.lua` but not the
/// module it requires shipped, raising on every single start.
#[test]
fn an_installed_neotmux_starts_on_its_shipped_layer() {
    let prefix = tempfile::tempdir().expect("prefix");
    install_into(prefix.path());

    let mut nt = Neotmux::start_installed(prefix.path(), "cat");
    let status = nt.wait_for_status("cat", PATIENCE);
    assert!(
        status.starts_with(CAP_LEFT),
        "the installed bar is missing its glyphs: {status:?}"
    );

    // Out of the box, with no user Lua at all: the shipped prefix is the only way to reach
    // a second pane, so this is what v0.1's line costs an installed user.
    nt.send("\x02v"); // C-b v
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);

    // No user Lua means no loosened budget, and an overrun on a loaded runner is the runner's.
    let log = nt.log_text();
    let failures: Vec<&str> = log
        .lines()
        .filter(|line| {
            !line.contains("tick exceeded its") && !line.contains("overran its tick budget")
        })
        .collect();
    assert!(
        failures.is_empty(),
        "the installed layer reported a failure:\n{log}"
    );
}

/// An event must describe the session as it is. Before this, `pane_closed` and
/// `focus_changed` were dispatched at the top of the frame against the geometry published
/// for the *previous* one, so a handler still saw the dead pane — `pane.current()` included.
#[test]
fn a_pane_event_handler_sees_the_tree_the_event_describes() {
    let (mut nt, _init) = start("require('plugin.splitter')", "cat");

    nt.send("\x02"); // C-b: split
    let log = nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    assert!(
        line_with(&log, "splitter pane_opened 2").contains("panes=2 current=2"),
        "a pane_opened handler could not see the pane it was told about:\n{log}"
    );

    nt.send("\x04"); // C-d: EOF for the focused pane's cat
    let log = nt.wait_for_log("splitter pane_closed 2", PATIENCE);
    assert!(
        line_with(&log, "splitter pane_closed 2").contains("panes=1 current=1"),
        "a pane_closed handler still saw the dead pane:\n{log}"
    );
}

/// M1b makes a tick attributable, but a split is applied after the tick: 200 `pane.split`
/// ops pushed from one callback had the host fork a shell for each, entirely outside any
/// source's measured slice. The cap is what keeps a plugin from buying unmeasured host work
/// by queueing it, and the refusal is reported against whoever asked.
#[test]
fn a_frame_applies_only_as_many_splits_as_it_can_afford() {
    let (mut nt, _init) = start(
        "require('plugin.splitter') require('plugin.flooder')",
        "cat",
    );

    nt.send("\x06"); // C-f: 200 splits from one callback
    let log = nt.wait_for_log("new shells per frame", PATIENCE);
    let refusal = line_with(&log, "new shells per frame");
    assert!(
        refusal.contains("plugin/flooder"),
        "the refusal did not name the source that asked: {refusal}"
    );

    nt.send("\x07"); // C-g: report the pane list
    let log = nt.wait_for_log("splitter panes=", PATIENCE);
    let listed = line_with(&log, "splitter panes=");
    assert!(
        listed.contains("panes=4"),
        "the frame forked more shells than its cap allows: {listed}"
    );
}

/// Every `resized` the probe plugin logged, in order.
fn resizes(log: &str) -> Vec<&str> {
    log.lines()
        .filter(|line| line.contains("resized "))
        .collect()
}

/// The probe: the shipped status line, a plugin that logs every `resized`, and one that
/// splits and walks the focus on a keystroke.
fn resize_probe() -> (Neotmux, tempfile::TempDir) {
    let init = harness::runtime(&[(
        "init.lua",
        "require('plugin.splitter') require('plugin.resizes')",
    )]);
    let nt = Neotmux::start_layered(
        &[init.path(), &harness::fixture_layer(), &shipped_layer()],
        "cat",
    );
    (nt, init)
}

/// A split costs one layout recompute — the one that made room for the new pane's siblings —
/// and no resize at all. The epoch the frame's draw buffers were committed under used to be
/// the same value the geometry published to Lua carried, so republishing it mid-frame to give
/// `pane_opened` a true tree invalidated the status that the tick had already committed. The
/// host read the empty status as "the status plugin stopped drawing", took its row back and
/// gave it straight back on the next frame: two recomputes and two SIGWINCHes per split.
#[test]
fn a_split_does_not_resize_every_pane() {
    let (mut nt, _init) = resize_probe();
    nt.wait_for_status("cat", PATIENCE);
    // Startup's own resizes have to be over before counting, and none of them is the last.
    nt.settle(Duration::from_millis(500));
    let before = resizes(&nt.log_text()).len();

    nt.send("\x02"); // C-b: split, focusing the new pane
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    // A resize, the bug, would come after the split the wait saw, and has no marker of its own.
    nt.settle(Duration::from_millis(500));

    let log = nt.log_text();
    let after = resizes(&log);
    assert_eq!(
        after.len(),
        before,
        "the split resized every pane: the status row blanked for a frame, so the host took \
         its row back and gave it straight back. Unexpected: {:?}",
        &after[before.min(after.len())..]
    );
    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        nt.status().contains("cat"),
        "the status row did not survive the split:\n{screen}"
    );
}

/// A terminal resize arrives as `SIGWINCH` and as nothing else: no byte is sent on the
/// terminal to announce one. Neotmux reads its own input now, so nothing else is listening
/// for that signal on its behalf.
#[test]
fn a_terminal_resize_reaches_the_panes() {
    let (mut nt, _init) = resize_probe();
    nt.wait_for_status("cat", PATIENCE);

    nt.resize(30, 100);

    // One row of the thirty belongs to the shipped status bar.
    nt.wait_for_log("resized 29x100", PATIENCE);
}

/// The same for a focus change, which invalidated the epoch for the same reason: focus is one
/// of the three things it is made of.
#[test]
fn a_focus_change_does_not_resize_every_pane() {
    let (mut nt, _init) = resize_probe();
    nt.wait_for_status("cat", PATIENCE);
    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    // The split's own resizes have to be over before counting, and none of them is the last.
    nt.settle(Duration::from_millis(500));
    let before = resizes(&nt.log_text()).len();

    nt.send("\x0f"); // C-o: focus the other pane
    nt.wait_for_log("splitter focus_changed 1", PATIENCE);
    // A resize, the bug, would come after the focus change the wait saw, and has no marker.
    nt.settle(Duration::from_millis(500));

    let log = nt.log_text();
    let after = resizes(&log);
    assert_eq!(
        after.len(),
        before,
        "the focus change resized every pane: the status row blanked for a frame, so the host \
         took its row back and gave it straight back. Unexpected: {:?}",
        &after[before.min(after.len())..]
    );
    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        nt.status().contains("cat"),
        "the status row did not survive the focus change:\n{screen}"
    );
}

/// What `pane.info` reports is the pane's *foreground* process, not the shell neotmux
/// spawned: a user running `sleep` wants to read `sleep`, and reads the shell again the
/// moment it finishes. Reporting the spawned child instead would say `bash` throughout, and
/// that is the whole distinction this test exists to pin.
#[test]
fn a_panes_command_follows_what_it_is_running_and_falls_back_to_its_shell() {
    let (mut nt, _init) = start("require('plugin.paneinfo')", "bash");
    nt.wait_for_log("paneinfo 1 command=bash", PATIENCE);

    nt.send("sleep 2\n");
    nt.wait_for_log("paneinfo 1 command=sleep", PATIENCE);

    // After the `sleep` line, not anywhere in the log: the shell was already reported once
    // before the command ever ran, and that earlier line proves nothing.
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let log = nt.log_text();
        let (_, after) = log
            .split_once("command=sleep")
            .expect("the sleep was reported");
        if after.contains("command=bash") {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the pane never reported its shell again once the command finished:\n{log}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_panes_directory_follows_a_cd_in_it() {
    let (mut nt, _init) = start("require('plugin.paneinfo')", "bash");
    nt.wait_for_log("paneinfo 1 command=bash", PATIENCE);

    let target = std::fs::canonicalize("/tmp").expect("canonicalise /tmp");
    nt.send(&format!("cd {}\n", target.display()));

    let log = nt.wait_for_log(&format!("cwd={}", target.display()), PATIENCE);
    assert!(
        line_with(&log, &format!("cwd={}", target.display())).contains("paneinfo 1"),
        "{log}"
    );
}

/// Where the first pane begins: the directory neotmux was started from, not the user's home
/// — the one a person means when they drop a multiplexer into a checkout. The home
/// directory is the isolation tempdir the harness hands the session, and it is *different*
/// from `launch`, which is what makes the timeout below a failure rather than a flake.
#[test]
fn the_first_pane_begins_where_neotmux_was_started() {
    let launch = tempfile::tempdir().expect("launch dir");
    let home = std::fs::canonicalize(launch.path()).expect("canonical launch");
    let (nt, _init) = start_in("require('plugin.paneinfo')", "bash", launch.path());

    let log = nt.wait_for_log("paneinfo 1 command=bash", PATIENCE);
    assert!(
        line_with(&log, "paneinfo 1 command=bash").contains(&format!("cwd={}", home.display())),
        "{log}"
    );
}

/// Where a split begins: where the focused pane itself is, the way the
/// `#{pane_current_path}` idiom means it — the pane's own directory, read from the process
/// running in it, not the directory neotmux came from. The new pane is id 2 and the
/// assertion is on *its* line: pane 1 already reports the target directory from the `cd`,
/// so the pane number is what carries the claim.
#[test]
fn the_split_begins_in_the_focused_panes_directory() {
    let launch = tempfile::tempdir().expect("launch dir");
    let home = std::fs::canonicalize(launch.path()).expect("canonical launch");
    let target = home.join("elsewhere");
    std::fs::create_dir(&target).expect("subdir");

    let (mut nt, _init) = start_in(
        "require('plugin.paneinfo') require('plugin.splitter')",
        "bash",
        launch.path(),
    );
    nt.wait_for_log(
        &format!("paneinfo 1 command=bash cwd={}", home.display()),
        PATIENCE,
    );

    nt.send(&format!("cd {}\n", target.display()));
    nt.wait_for_log(&format!("cwd={}", target.display()), PATIENCE);

    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    let log = nt.wait_for_log("paneinfo 2 ", PATIENCE);
    assert!(
        line_with(&log, "paneinfo 2 ").contains(&format!("cwd={}", target.display())),
        "{log}"
    );
}

/// The split's fallback: the focused pane's directory is gone — deleted out from under it —
/// so the only directory left to answer with is where neotmux was started. Not `$HOME`,
/// which is what the un-pinned path silently chooses, and not an error, which would cost
/// the split.
#[test]
fn the_split_whose_focused_pane_left_a_deleted_directory_begins_where_neotmux_was_started() {
    let launch = tempfile::tempdir().expect("launch dir");
    let home = std::fs::canonicalize(launch.path()).expect("canonical launch");
    let vanished = home.join("vanished");
    std::fs::create_dir(&vanished).expect("subdir");

    let (mut nt, _init) = start_in(
        "require('plugin.paneinfo') require('plugin.splitter')",
        "bash",
        launch.path(),
    );
    nt.wait_for_log(
        &format!("paneinfo 1 command=bash cwd={}", home.display()),
        PATIENCE,
    );

    nt.send(&format!("cd {}\n", vanished.display()));
    nt.wait_for_log(&format!("cwd={}", vanished.display()), PATIENCE);
    std::fs::remove_dir_all(&vanished).expect("delete out from under the pane");

    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    let log = nt.wait_for_log("paneinfo 2 ", PATIENCE);
    assert!(
        line_with(&log, "paneinfo 2 ").contains(&format!("cwd={}", home.display())),
        "{log}"
    );
}

/// An id no pane has is not an error — a plugin holding a stale id asks the same question as
/// one holding a live one. Nil, the way `pane.current()` answers nothing.
#[test]
fn an_unknown_pane_id_answers_nil_rather_than_raising() {
    let (nt, _init) = start("require('plugin.paneinfo')", "/bin/cat");
    let log = nt.wait_for_log("paneinfo unknown=", PATIENCE);
    assert!(
        line_with(&log, "paneinfo unknown=").contains("unknown=nil"),
        "{log}"
    );
    assert!(!log.contains("[ERROR]"), "{log}");
}

/// A pane's child can exit between one tick and the next. That is an ordinary end, not an
/// error, and the plugin asking about the panes that are left must not start raising.
#[test]
fn a_pane_whose_child_exited_does_not_raise() {
    let (mut nt, _init) = start(
        "require('plugin.paneinfo') require('plugin.splitter')",
        "bash",
    );
    nt.wait_for_log("paneinfo 1 command=bash", PATIENCE);
    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    nt.wait_for_log("paneinfo 2 command=bash", PATIENCE);

    nt.send("exit\n");
    nt.wait_for_log("splitter pane_closed 2", PATIENCE);
    // Room for the ticks after the close to raise, if they were going to: nothing to wait for.
    nt.settle(Duration::from_millis(500));

    let log = nt.log_text();
    assert!(!log.contains("[ERROR]"), "{log}");
    // Still answering for the pane that is left, rather than having quietly stopped.
    nt.send("sleep 1\n");
    nt.wait_for_log("paneinfo 1 command=sleep", PATIENCE);
}

/// A plugin reading both facts every tick must not turn an idle session into one that
/// redraws: the facts are sampled on the host's own cadence, and a sample that found nothing
/// new produces no output at all.
#[test]
fn a_plugin_asking_every_tick_does_not_make_an_idle_session_redraw() {
    let (mut nt, _init) = start("require('plugin.paneinfo')", "/bin/cat");
    nt.wait_for_log("paneinfo 1 command=cat", PATIENCE);
    // Startup's own frames have no last one to wait for; they are over by the time this is.
    nt.settle(Duration::from_millis(700));

    let bytes = nt.bytes_in(Duration::from_secs(1));
    assert_eq!(bytes, 0, "an idle session redrew {bytes} bytes");
}

/// M2b: windows. A window is an independent pane tree, and switching shows a different one
/// while the panes of the others go on running.
///
/// Both fixture plugins are loaded together where a test needs a split as well as a switch:
/// `plugin.splitter` owns `C-b`/`C-o`/`C-g`, `plugin.windows` owns `C-w`/`C-t`/`C-n`/`C-r`.
const WINDOW_FIXTURES: &str = "require('plugin.splitter') require('plugin.windows')";

/// Switching shows a different pane, and switching back shows the first one's content
/// exactly as it was left.
#[test]
fn a_second_window_shows_its_own_pane_and_the_first_keeps_its_content() {
    let (mut nt, _init) = start("require('plugin.windows')", "cat");
    nt.send("FIRST\r");
    nt.wait_for_screen("FIRST", PATIENCE);

    nt.send("\x17"); // C-w: open a window and show it
    nt.wait_for_log("windows window_changed 2", PATIENCE);
    nt.wait_until(
        "the new window is showing the old window's pane",
        PATIENCE,
        |nt| !nt.screen_contents().contains("FIRST"),
    );

    nt.send("SECOND");
    nt.wait_for_screen("SECOND", PATIENCE);

    nt.send("\x0e"); // C-n: back to the first window
    nt.wait_for_log("windows window_changed 1", PATIENCE);
    nt.wait_for_screen("FIRST", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        !screen.contains("SECOND"),
        "the first window is showing the second window's pane:\n{screen}"
    );
}

/// The whole point of a window nobody is looking at: its shell goes on working, and its
/// output is read while it is hidden rather than caught up with on the way back — a pane
/// whose reader nobody empties eventually blocks its child.
///
/// The marker is computed by the shell rather than typed, so the echo of the command line
/// cannot be mistaken for the output of running it.
#[test]
fn a_command_run_in_a_hidden_window_has_produced_its_output_by_the_time_you_switch_back() {
    let (mut nt, _init) = start("require('plugin.windows')", "/bin/sh");
    nt.send("echo UP\r");
    nt.wait_for_screen("UP", PATIENCE);
    nt.send("\x17"); // C-w: a second window, shown
    nt.wait_for_log("windows window_changed 2", PATIENCE);

    nt.send("sleep 1; echo LATE$((6*7))\r");
    nt.send("\x0e"); // C-n: away before the sleep is over
    nt.wait_for_log("windows window_changed 1", PATIENCE);

    let screen = nt.screen_after(Duration::from_secs(2));
    assert!(
        !screen.contains("LATE42"),
        "the hidden window's output landed in the window being shown:\n{screen}"
    );

    // Everything the log has to say between leaving the second window and going back to it.
    let log = nt.log_text();
    let while_hidden: Vec<&str> = log
        .lines()
        .skip_while(|line| !line.contains("window_changed 1"))
        .collect();
    assert!(
        while_hidden
            .iter()
            .any(|line| line.contains("windows output 2")),
        "the hidden pane's output was not read while it was hidden:\n{log}"
    );

    nt.send("\x0e"); // C-n: back to it, well after the output was produced
    nt.wait_for_log("windows window_changed 2", PATIENCE);
    nt.wait_for_screen("LATE42", PATIENCE);
}

/// A pane in a hidden window is still a pane: two trees numbering their panes apart would
/// make `pane.focus(id)` name two of them.
#[test]
fn pane_ids_do_not_collide_across_windows() {
    let (mut nt, _init) = start(WINDOW_FIXTURES, "cat");
    nt.send("UP\r");
    nt.wait_for_screen("UP", PATIENCE);
    nt.send("\x02"); // C-b: split the first window
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    nt.send("\x17"); // C-w: a second window
    nt.wait_for_log("windows window_changed 2", PATIENCE);
    nt.send("\x02"); // C-b: split it too
    nt.wait_for_log("splitter pane_opened 4", PATIENCE);

    nt.send("\x12"); // C-r: report
    let log = nt.wait_for_log("windows report", PATIENCE);
    let report = line_with(&log, "windows report");
    assert!(
        report.contains("windows=1,2 current=2 panes=3,4"),
        "the second window reused the first window's pane ids: {report}"
    );

    nt.send("\x0e"); // C-n: back to the first
    nt.wait_for_log("windows window_changed 1", PATIENCE);
    nt.send("\x12");
    nt.wait_until("the second report never came", PATIENCE, |nt| {
        nt.log_text().matches("windows report").count() >= 2
    });
    let log = nt.log_text();
    let reports: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("windows report"))
        .collect();
    assert!(
        reports.last().expect("a report").contains("panes=1,2"),
        "the first window's panes are not its own: {log}"
    );
}

/// `pane.list` answers about the window being shown, so a hidden window's pane is not in it
/// — but `pane.info` describes it all the same. What the pane in the window you are not
/// looking at is running is the question a sidebar exists to answer.
#[test]
fn a_hidden_windows_pane_is_in_no_list_but_still_reports_what_it_is_running() {
    let (mut nt, _init) = start(
        "require('plugin.windows') require('plugin.paneinfo')",
        "/bin/cat",
    );
    nt.wait_for_log("paneinfo 1 command=cat", PATIENCE);
    nt.send("UP\r");
    nt.wait_for_screen("UP", PATIENCE);
    nt.send("\x17"); // C-w: a second window, shown; the first window's pane is now hidden
    nt.wait_for_log("windows window_changed 2", PATIENCE);
    nt.send("\x12"); // C-r: what the shown window's `pane.list` holds
    nt.wait_for_log("windows report", PATIENCE);
    // Past the host's sampling cadence, so a sample that dropped the hidden pane has been taken.
    nt.settle(Duration::from_millis(700));

    let log = nt.log_text();
    let report = line_with(&log, "windows report");
    assert!(
        report.contains("panes=2"),
        "the hidden window's pane is in the shown window's list: {report}"
    );
    // The last word on each pane, not any line about it: the hidden one was described before
    // it was hidden, and a sample that dropped it would say so afterwards.
    let latest = |pane: &str| {
        let marker = format!("paneinfo {pane} ");
        log.lines()
            .rfind(|line| line.contains(&marker))
            .unwrap_or_else(|| panic!("nothing was ever logged about pane {pane}:\n{log}"))
            .to_string()
    };
    assert!(
        latest("2").contains("command=cat cwd=/"),
        "the shown window's pane has no info: {}",
        latest("2")
    );
    assert!(
        latest("1").contains("command=cat cwd=/"),
        "the hidden window's pane stopped reporting its command and directory: {}",
        latest("1")
    );
}

/// The facts are sampled while the pane is hidden, not caught up with when it is shown again:
/// a `cd` and a command started after the window went away are both reported.
#[test]
fn a_hidden_panes_command_and_directory_follow_what_it_does_while_it_is_hidden() {
    let (mut nt, _init) = start(
        "require('plugin.windows') require('plugin.paneinfo')",
        "bash",
    );
    nt.wait_for_log("paneinfo 1 command=bash", PATIENCE);

    // Both happen a second after the line is sent, so neither can have been sampled before
    // the window was hidden.
    let target = std::fs::canonicalize("/tmp").expect("canonicalise /tmp");
    nt.send(&format!(
        "sleep 1; cd {}; tail -f /dev/null\n",
        target.display()
    ));
    nt.send("\x17"); // C-w: a second window, shown; the first window's pane is now hidden
    nt.wait_for_log("windows window_changed 2", PATIENCE);

    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let log = nt.log_text();
        let (_, hidden) = log
            .split_once("window_changed 2")
            .expect("the window was hidden");
        let found = hidden.lines().any(|line| {
            line.contains("paneinfo 1 command=tail")
                && line.contains(&format!("cwd={}", target.display()))
        });
        if found {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the hidden pane never reported the command and directory it took on:\n{log}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Every pane is sampled now, so a pane that closed must leave the sample rather than sit in
/// it answering with whatever it was running when it died.
#[test]
fn a_closed_pane_leaves_no_stale_info_behind() {
    let (mut nt, _init) = start(
        "require('plugin.splitter') require('plugin.paneinfo')",
        "bash",
    );
    nt.wait_for_log("paneinfo 1 command=bash", PATIENCE);
    nt.send("\x02"); // C-b: split, focusing the new pane
    nt.wait_for_log("paneinfo 2 command=bash", PATIENCE);

    nt.send("exit\n");
    let log = nt.wait_for_log("paneinfo gone 2=", PATIENCE);
    assert!(
        line_with(&log, "paneinfo gone 2=").contains("gone 2=nil"),
        "a closed pane is still being described:\n{log}"
    );
}

/// The last pane of a window closing closes the window, and what is shown moves to another.
#[test]
fn closing_the_last_pane_of_a_window_closes_it_and_shows_another() {
    let (mut nt, _init) = start("require('plugin.windows')", "cat");
    nt.send("UP\r");
    nt.wait_for_screen("UP", PATIENCE);
    nt.send("\x17"); // C-w
    nt.wait_for_log("windows window_changed 2", PATIENCE);

    nt.send("\x04"); // C-d: EOF for the shown pane's cat
    let log = nt.wait_for_log("windows window_closed 2", PATIENCE);
    assert!(
        line_with(&log, "window_closed 2").contains("windows=1"),
        "a window_closed handler still saw the dead window:\n{log}"
    );
    nt.wait_for_log("windows window_changed 1", PATIENCE);

    nt.send("SURVIVOR");
    nt.wait_for_screen("SURVIVOR", PATIENCE);
}

/// M0a's guarantee, with windows in the way: the last window's last pane ends the session,
/// and it ends cleanly.
#[test]
fn closing_the_last_window_exits_cleanly() {
    let (mut nt, _init) = start("require('plugin.windows')", "cat");
    nt.send("UP\r");
    nt.wait_for_screen("UP", PATIENCE);
    nt.send("\x17");
    nt.wait_for_log("windows window_changed 2", PATIENCE);

    nt.send("\x04"); // the second window's only pane
    nt.wait_for_log("windows window_closed 2", PATIENCE);

    nt.send("\x04"); // and the first window's, which is the last pane there is
    assert!(nt.exited_cleanly(PATIENCE));
}

/// Focusing a window that does not exist is reported, and the session lives on.
#[test]
fn focusing_an_unknown_window_is_reported_and_does_not_stop_the_session() {
    let (mut nt, _init) = start("require('plugin.windows')", "cat");
    nt.send("UP\r");
    nt.wait_for_screen("UP", PATIENCE);

    nt.send("\x19"); // C-y: focus a window id nobody owns
    let log = nt.wait_for_log("window.focus", PATIENCE);
    let refusal = line_with(&log, "window.focus");
    assert!(refusal.contains("plugin/windows"), "{refusal}");
    assert!(refusal.contains("no such window"), "{refusal}");

    nt.send("ALIVE");
    nt.wait_for_screen("ALIVE", PATIENCE);
}

/// A band belongs to the surface, not to a window: the status plugin reserved it once and
/// keeps it across a switch.
#[test]
fn the_shipped_status_band_survives_a_switch() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);

    nt.send("\x02c"); // C-b c: a window, shown
    nt.wait_for_status(SECOND_WINDOW, PATIENCE);
    assert_eq!(
        nt.cell(0, 0),
        CAP_LEFT,
        "the status bar left the row it reserved:\n{}",
        nt.screen_after(Duration::ZERO)
    );

    nt.send("\x02n"); // C-b n: back
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);
    assert_eq!(
        nt.cell(0, 0),
        CAP_LEFT,
        "the status bar lost its band on the way back:\n{}",
        nt.screen_after(Duration::ZERO)
    );
}

/// `C-b p` is the step `C-b n` undoes. Three windows is the smallest number that tells them
/// apart: from the third, `n` wraps to the first and `p` goes back to the second.
#[test]
fn the_prefix_walks_back_to_the_window_before() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);
    nt.send("\x02c"); // C-b c
    nt.wait_for_status(SECOND_WINDOW, PATIENCE);
    nt.send("\x02c"); // C-b c
    nt.wait_for_status(THIRD_WINDOW, PATIENCE);

    nt.send("\x02p"); // C-b p
    nt.wait_for_status(SECOND_WINDOW, PATIENCE);

    // And it wraps at the near end the way `n` wraps at the far one.
    nt.send("\x02p"); // C-b p
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);
    nt.send("\x02p"); // C-b p
    nt.wait_for_status(THIRD_WINDOW, PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        !pane_area(&screen).contains("^B"),
        "a prefix keystroke reached a shell:\n{screen}"
    );
}

/// `C-b c` and `C-b n` do what they say, expressed with nothing but `neotmux.window`.
#[test]
fn the_prefix_opens_a_window_and_walks_to_the_next_one() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);
    nt.send("ONE");
    nt.wait_for_screen("ONE", PATIENCE);

    nt.send("\x02c"); // C-b c
    nt.wait_for_status(SECOND_WINDOW, PATIENCE);
    nt.send("TWO");
    nt.wait_for_screen("TWO", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(300));
    assert!(
        !pane_area(&screen).contains("ONE"),
        "`C-b c` did not open a window of its own:\n{screen}"
    );

    nt.send("\x02n"); // C-b n
    nt.wait_for_status(FIRST_WINDOW, PATIENCE);
    nt.wait_for_screen("ONE", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(300));
    let panes = pane_area(&screen);
    assert!(
        !panes.contains("TWO"),
        "`C-b n` did not walk to the other window:\n{screen}"
    );
    assert!(
        !panes.contains("^B"),
        "a prefix keystroke reached a shell:\n{screen}"
    );
}

/// A window opened without being shown is a window running out of sight: `create()` alone
/// does not switch, which is what makes `focus(create())` the thing that does.
#[test]
fn creating_a_window_without_focusing_it_leaves_it_hidden() {
    let (mut nt, _init) = start("require('plugin.windows')", "cat");
    nt.send("FIRST\r");
    nt.wait_for_screen("FIRST", PATIENCE);

    nt.send("\x14"); // C-t: create only
    let log = nt.wait_for_log("windows window_opened 2", PATIENCE);
    assert!(
        line_with(&log, "window_opened 2").contains("windows=1,2 current=1 panes=1"),
        "creating a window changed what is shown:\n{log}"
    );

    let screen = nt.screen_after(Duration::from_millis(500));
    assert!(
        screen.contains("FIRST"),
        "the shown window's pane was replaced by one nobody asked to see:\n{screen}"
    );
}

/// M3: reading what a pane is showing. `plugin.content` reads every pane it has heard output
/// from, out of a tick and out of the `pane_output` handler the chunk arrived in.
/// A "shell" that waits for a file to appear, says one line, and then behaves like `cat`.
/// Typing only ever reaches the focused pane, so this is how a pane is hidden first and made
/// to produce output afterwards — which is the only pane a watcher is really for.
fn triggered_shell(dir: &Path, trigger: &Path, says: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = dir.join("triggered-shell");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nwhile [ ! -f '{}' ]; do sleep 0.05; done\nprintf '%s\\n' '{says}'\nexec cat\n",
            trigger.display()
        ),
    )
    .expect("write the shell");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

/// The first log line holding every one of `parts`, waited for. `wait_for_log` takes one
/// marker, and these tests are about a particular pane's line among several panes'.
fn wait_for_log_line(nt: &Neotmux, parts: &[&str]) -> String {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let log = nt.log_text();
        if let Some(line) = log
            .lines()
            .find(|line| parts.iter().all(|part| line.contains(part)))
        {
            return line.to_string();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no log line holds all of {parts:?}:\n{log}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Lines, not bytes: what the child wrote, with the terminal already reconstructed, so a
/// plugin matching a pattern is never handed an escape sequence to strip.
#[test]
fn a_panes_content_is_what_was_written_to_it_as_lines() {
    let (mut nt, _init) = start("require('plugin.content')", "/bin/cat");
    // `cat` echoes what is typed and then says it again, so the pane shows two rows.
    nt.send("HELLO\r");

    let line = wait_for_log_line(&nt, &["content 1 tick", "[HELLO|HELLO]"]);
    assert!(!line.contains('\u{1b}'), "an escape survived into {line:?}");
    // The chunk has reached the pane's terminal before the event carrying it is dispatched.
    wait_for_log_line(&nt, &["content 1 output", "HELLO"]);
}

/// An id no pane has is not an error, and a watcher loop must not stop over one.
#[test]
fn an_unknown_pane_id_has_no_content_rather_than_raising() {
    let (nt, _init) = start("require('plugin.content')", "/bin/cat");
    let log = nt.wait_for_log("content unknown=", PATIENCE);
    assert!(
        line_with(&log, "content unknown=").contains("unknown=nil"),
        "{log}"
    );
    assert!(!log.contains("[ERROR]"), "{log}");
}

/// A shell exiting between one pass of the loop and the next is an ordinary end.
#[test]
fn a_pane_that_closes_while_being_read_does_not_raise() {
    let (mut nt, _init) = start(
        "require('plugin.content') require('plugin.splitter')",
        "/bin/cat",
    );
    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    nt.send("DOOMED\r");
    wait_for_log_line(&nt, &["content 2 tick", "DOOMED"]);

    nt.send("\x04"); // C-d: EOF for the focused pane's cat
    nt.wait_for_log("splitter pane_closed 2", PATIENCE);
    // The pane does not exist any more, so nothing may still be reading its terminal.
    let log = nt.wait_for_log("content 2 closed=", PATIENCE);
    assert!(
        line_with(&log, "content 2 closed=").contains("closed=nil"),
        "a dead pane was still showing something:\n{log}"
    );
    // Room for the ticks after the close to raise, if they were going to: nothing to wait for.
    nt.settle(Duration::from_millis(500));
    assert!(!nt.log_text().contains("[ERROR]"), "{}", nt.log_text());

    // Still reading the pane that is left, rather than having quietly stopped.
    nt.send("SURVIVOR\r");
    wait_for_log_line(&nt, &["content 1 tick", "SURVIVOR"]);
}

/// The whole point. A pane in a window nobody is looking at is in no `pane.list()`, its shell
/// is running anyway, and what it is showing is exactly the question worth asking — including
/// what it said while it was hidden.
#[test]
fn a_hidden_windows_pane_can_be_read_and_reflects_output_that_arrived_while_hidden() {
    let scratch = tempfile::tempdir().expect("scratch dir");
    let trigger = scratch.path().join("go");
    let shell = triggered_shell(scratch.path(), &trigger, "SPOKE-WHILE-HIDDEN");
    let dir = harness::runtime(&[(
        "init.lua",
        "require('plugin.content') require('plugin.windows')",
    )]);
    let fixtures = harness::fixture_layer();
    let mut nt = Neotmux::start_layered(
        &[dir.path(), &fixtures],
        shell.to_str().expect("a utf-8 path"),
    );

    // The session has to be in raw mode before `C-w` is sent, or the line discipline eats it
    // as a word erase. The first tick is over by then.
    nt.wait_for_log("content unknown=", PATIENCE);
    nt.send("\x17"); // C-w: a second window, shown; window 1's pane is now hidden
    nt.wait_for_log("windows window_changed 2", PATIENCE);
    // The hidden pane is in no list, so nothing here could have come from one.
    let report = {
        nt.send("\x12"); // C-r
        wait_for_log_line(&nt, &["windows report"])
    };
    assert!(
        report.contains("windows=1,2 current=2 panes=2"),
        "pane 1 was not hidden: {report}"
    );

    std::fs::write(&trigger, "").expect("trigger the shells");

    let line = wait_for_log_line(&nt, &["content 1 ", "SPOKE-WHILE-HIDDEN"]);
    assert!(
        line.contains("rev="),
        "a hidden pane answered without a revision: {line}"
    );
}

/// The herdr job, end to end and over a PTY: the agent you are *not* looking at is the one
/// that has to tell you it is waiting.
#[test]
fn the_shipped_watcher_says_when_a_hidden_pane_is_waiting_for_you() {
    let scratch = tempfile::tempdir().expect("scratch dir");
    let trigger = scratch.path().join("go");
    let shell = triggered_shell(scratch.path(), &trigger, "Do you want to proceed? [y/N]");
    let init = harness::runtime(&[("init.lua", "require('neotmux.defaults').watch = true\n")]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &shipped_layer()],
        shell.to_str().expect("a utf-8 path"),
    );

    nt.wait_for_screen("1.1", PATIENCE);
    nt.send("\x02c"); // C-b c: a second window, shown; pane 1 is now hidden
    nt.wait_for_screen("2.1", PATIENCE);

    // Both panes run the same shell, so both say the same thing at the same moment.
    std::fs::write(&trigger, "").expect("trigger the shells");

    nt.wait_for_log("pane 1 looks like it is waiting for you", PATIENCE);
    // Room for a notification about pane 2 to arrive, if it were going to: nothing to wait for.
    nt.settle(Duration::from_millis(500));
    assert!(
        !nt.log_text().contains("pane 2 looks like"),
        "the pane in front of the user was reported anyway:\n{}",
        nt.log_text()
    );
}

/// Off unless the user says otherwise: the watcher is an opinion about what is being run,
/// which the other shipped plugins are not.
#[test]
fn the_shipped_watcher_is_off_by_default() {
    let scratch = tempfile::tempdir().expect("scratch dir");
    let trigger = scratch.path().join("go");
    let shell = triggered_shell(scratch.path(), &trigger, "Do you want to proceed? [y/N]");
    let init = harness::runtime(&[("init.lua", "")]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &shipped_layer()],
        shell.to_str().expect("a utf-8 path"),
    );

    nt.wait_for_screen("1.1", PATIENCE);
    std::fs::write(&trigger, "").expect("trigger the shell");
    nt.wait_for_screen("Do you want to proceed?", PATIENCE);
    // Room for a notification to arrive, if it were going to: nothing to wait for.
    nt.settle(Duration::from_millis(500));
    assert!(
        !nt.log_text().contains("looks like it is waiting"),
        "a plugin nobody asked for was running:\n{}",
        nt.log_text()
    );
}

/// `plugin/sidebar.lua`'s width, herdr's: 25 columns and the rule down its edge.
const SIDEBAR_COLS: u16 = 26;
/// The state icon against a session: idle, working, or waiting for you. Which one it is
/// depends on what the shell said and when, so a test that only cares that the band is there
/// asks whether the cell holds any of them.
const IDLE_SESSION_ICON: &str = "\u{25cb}";
const SESSION_ICONS: [&str; 3] = ["\u{25cb}", "\u{25cf}", "\u{25c9}"];

fn is_a_state_icon(cell: &str) -> bool {
    SESSION_ICONS.contains(&cell)
}
const CURRENT_WINDOW_MARKER: &str = "\u{25b8}";

/// The tree of the session you are in, which these tests are about: herdr's spaces are the
/// shipped default now, and the tree is still shipped, one assignment away — shared out by what
/// each section has rather than cut at half, since a tree wants the rows an idle agent list is
/// not using.
const TREE_SIDEBAR: &str = "require('neotmux.defaults').sidebar = true\n\
    local sidebar = require('plugin.sidebar')\n\
    sidebar.sections = { require('plugin.sidebar.sessions'), require('plugin.sidebar.agents') }\n\
    sidebar.split = nil";

/// The sidebar's rows are two columns — a label on the left and what the thing is running
/// against the right edge — so what names a pane is one row holding both, not one string.
fn sidebar_row_naming(nt: &mut Neotmux, label: &str, command: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let screen = nt.screen_after(Duration::from_millis(20));
        if screen
            .lines()
            .any(|line| line.contains(label) && line.contains(command))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no sidebar row named {label} as running {command}:\n{screen}"
        );
    }
}

/// The vertical edges end to end, on the screen a user sees: the sidebar is a shipped plugin
/// that asks for a band of columns, lists what the session holds in it, and costs the panes
/// exactly the columns it took.
#[test]
fn the_shipped_sidebar_lists_the_session_and_narrows_the_panes() {
    let init = harness::runtime(&[("init.lua", TREE_SIDEBAR)]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");

    nt.wait_for_status("cat", PATIENCE);
    nt.wait_for_cell(PANE_TOP, 0, IDLE_SESSION_ICON, PATIENCE);
    // A session is two rows — its name, then what it is running — and its windows hang under
    // it, because that is where they are: a window belongs to a session and a pane to a
    // window, and the sidebar draws what the thing is.
    nt.wait_for_cell(PANE_TOP + 2, 0, CURRENT_WINDOW_MARKER, PATIENCE);
    sidebar_row_naming(&mut nt, "1.1", "cat", PATIENCE);

    // What the band costs: a pane starts where the sidebar ends, so the shell's own echo
    // lands in the first column outside it and in none before it.
    nt.send("X");
    nt.wait_for_cell(PANE_TOP, SIDEBAR_COLS, "X", PATIENCE);
    for col in 0..SIDEBAR_COLS {
        assert_ne!(
            nt.cell(PANE_TOP, col),
            "X",
            "the shell wrote into the band the sidebar reserved:\n{}",
            nt.screen_after(Duration::ZERO)
        );
    }

    nt.send("\x02c"); // C-b c: a second window, which the sidebar has to pick up
    sidebar_row_naming(&mut nt, "2.1", "cat", PATIENCE);
    // The first window's pane is hidden now, and naming what it is running is most of why a
    // sidebar is worth a third of the width.
    sidebar_row_naming(&mut nt, "1.1", "cat", PATIENCE);
}

/// What is left of the band when the sidebar is folded: herdr's rail, a number and a state.
const SIDEBAR_RAIL_COLS: u16 = 4;

/// The fold, on the screen a user sees: `C-b b` hands the columns back to the panes and
/// leaves the marker column behind, and pressing it again brings the sidebar back the width
/// it was and against the same edge. The echoed `X` is the ruler — it sits at the first
/// column the pane owns, so where it lands says how wide the band is.
#[test]
fn the_shipped_sidebar_folds_to_a_rail_and_back() {
    let init = harness::runtime(&[("init.lua", TREE_SIDEBAR)]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");

    nt.wait_for_status("cat", PATIENCE);
    sidebar_row_naming(&mut nt, "1.1", "cat", PATIENCE);
    nt.send("X");
    nt.wait_for_cell(PANE_TOP, SIDEBAR_COLS, "X", PATIENCE);

    nt.send("\x02b"); // C-b b
    nt.wait_for_cell(PANE_TOP, SIDEBAR_RAIL_COLS, "X", PATIENCE);
    // The pane moves on the frame the layout changes; the rail can be drawn a frame later.
    nt.wait_until("the fold took the markers column too", PATIENCE, |nt| {
        is_a_state_icon(&nt.cell(PANE_TOP, 0))
    });
    let folded = nt.screen_after(Duration::ZERO);
    // Below the status bar, which names the focused pane whatever the sidebar is doing.
    assert!(
        !folded
            .lines()
            .skip(PANE_TOP as usize)
            .any(|line| line.contains("1.1")),
        "the rail is too narrow to read and still had the names on it:\n{folded}"
    );

    nt.send("\x02b");
    nt.wait_for_cell(PANE_TOP, SIDEBAR_COLS, "X", PATIENCE);
    sidebar_row_naming(&mut nt, "1.1", "cat", PATIENCE);
    assert!(
        is_a_state_icon(&nt.cell(PANE_TOP, 0)),
        "the band came back somewhere other than the left edge:\n{}",
        nt.screen_after(Duration::ZERO)
    );
}

/// More rows than band is the ordinary case on a short terminal, and which rows go is the
/// question: the pane you are in has to be one of the ones that stayed, or the sidebar is
/// about somebody else's session.
#[test]
fn the_shipped_sidebar_keeps_the_pane_you_are_in_on_screen() {
    let init = harness::runtime(&[("init.lua", TREE_SIDEBAR)]);
    // Short on purpose: five windows do not fit in what is left of ten rows.
    let mut nt = Neotmux::start_sized(&[init.path(), &shipped_layer()], "cat", 10, 80);
    nt.wait_for_cell(PANE_TOP, 0, IDLE_SESSION_ICON, PATIENCE);

    for window in 2..=5 {
        nt.send("\x02c"); // C-b c
        nt.wait_for_status(&format!("{window}.1  cat"), PATIENCE);
    }

    // The window is the fifth, and its marker is on screen somewhere in the band.
    sidebar_row_naming(&mut nt, "5.1", "cat", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(200));
    assert!(
        screen
            .lines()
            .skip(PANE_TOP as usize)
            .any(|line| line.starts_with(CURRENT_WINDOW_MARKER)),
        "the window being shown scrolled off the bottom of the sidebar:\n{screen}"
    );
}

/// The shipped sidebar is herdr's: a `spaces` section heading every session, the one you are in
/// among them, and `C-b w`'s cursor over them, which Enter leaves the way it found them when
/// the session under it is the one you are in.
#[test]
fn the_shipped_sidebar_is_herdrs_spaces() {
    let init = harness::runtime(&[("init.lua", "require('neotmux.defaults').sidebar = true")]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");
    nt.wait_for_status("cat", PATIENCE);
    nt.wait_for_screen(" spaces", PATIENCE);
    nt.wait_for_screen(" default", PATIENCE);

    nt.send("\x02w"); // C-b w
    nt.send("\r"); // Enter: the session you are in, so nowhere to go
    nt.send("Y");
    nt.wait_for_cell(PANE_TOP, SIDEBAR_COLS, "Y", PATIENCE);
}

/// Off unless the user says otherwise, like the watcher: what the watcher spends is an
/// opinion about what you run, what a sidebar spends is a third of the width.
#[test]
fn the_shipped_sidebar_is_off_by_default() {
    let (mut nt, _init) = start_shipped("", "cat");

    nt.wait_for_status("cat", PATIENCE);
    nt.send("X");
    nt.wait_for_cell(PANE_TOP, 0, "X", PATIENCE);
}

/// Copy mode's own colours, as `plugin/copy.lua` sets them. A tint changes nothing a
/// `contents()` can see, so these are what the assertions below are made of.
const COPY_CURSOR: (u8, u8, u8) = (0xf9, 0xe2, 0xaf);
const COPY_SELECTION: (u8, u8, u8) = (0x45, 0x47, 0x5a);

/// Types a line at `cat` and waits for both copies of it: the one the tty echoes, which is on
/// screen before `cat` has even started, and the one `cat` writes back.
fn type_into_cat(nt: &mut Neotmux, line: &str) {
    nt.send(&format!("{line}\r"));
    nt.wait_until(&format!("cat never wrote {line:?} back"), PATIENCE, |nt| {
        nt.screen_contents().matches(line).count() >= 2
    });
}

/// The mode is entered on the pane you are in and changes nothing about it: the screen after
/// `C-b [` is the screen before it, its keys never reach the shell, and `q` hands the
/// keyboard back.
#[test]
fn copy_mode_leaves_the_pane_alone_and_gives_the_keys_back() {
    let (mut nt, _init) = start_shipped("", "cat");
    type_into_cat(&mut nt, "ABCDEFGH");
    // Both sides are read once the pane has settled: the mode snapshots the lines on the way
    // in, so a byte still in flight would legitimately make two frames differ.
    let before = settled_pane_area(&mut nt, PATIENCE);

    nt.send("\x02["); // C-b [
    // Every one of them a motion, and none of them a character. They end on the text, since a
    // cursor on an empty row is a cell `contents()` sees.
    nt.send("kkllhj");
    let inside = settled_pane_area(&mut nt, PATIENCE);
    assert_eq!(before, inside, "entering copy mode disturbed the pane");

    nt.send("q");
    nt.send("TYPED\r");
    nt.wait_for_screen("TYPED", PATIENCE);
    let log = nt.log_text();
    assert!(log.is_empty(), "copy mode reported a failure:\n{log}");
}

/// The same exit criterion for the one shipped default whose opt-out nothing else covers.
/// Off, `C-b [` opens nothing: the prefix is still loaded and still swallows both keys, but
/// no mode claims the keyboard afterwards, so what is typed next reaches the shell — which
/// is exactly what copy mode, when it is on, does not allow.
#[test]
fn a_user_config_can_switch_off_copy_mode() {
    let init = harness::runtime(&[("init.lua", "require('neotmux.defaults').copy = false")]);
    let shipped = shipped_layer();
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped], "cat");

    type_into_cat(&mut nt, "ABCDEFGH");

    nt.send("\x02["); // C-b [: nothing left to claim the bracket
    // Reaching the shell at all is the assertion: in copy mode these keys are motions.
    nt.send("TYPED\r");
    nt.wait_for_screen("TYPED", PATIENCE);
    assert!(
        nt.log_text().is_empty(),
        "switching copy mode off was reported as a failure:\n{}",
        nt.log_text()
    );
}

/// The cursor is a cell you can see, and the motion keys move it. Seen as a colour, because
/// a tint leaves the glyph exactly as it was.
#[test]
fn the_copy_cursor_is_visible_and_moves() {
    let (mut nt, _init) = start_shipped("", "cat");
    type_into_cat(&mut nt, "ABCDEFGH");

    nt.send("\x02["); // C-b [
    // The line the tty echoed and the line `cat` wrote back, and the pane's cursor under them.
    nt.wait_until(
        "the cursor is not where the pane's cursor was",
        PATIENCE,
        |nt| nt.cell_bg(PANE_TOP + 2, 0) == Some(COPY_CURSOR),
    );

    nt.send("lll");
    nt.send("k");
    nt.wait_until(
        "the cursor did not move three right and one up",
        PATIENCE,
        |nt| nt.cell_bg(PANE_TOP + 1, 3) == Some(COPY_CURSOR),
    );
    assert_ne!(
        nt.cell_bg(PANE_TOP + 2, 0),
        Some(COPY_CURSOR),
        "the cursor is in two places at once"
    );
}

/// The reason the mode exists: `g` goes to the oldest line the pane still holds, and the
/// lines the shell scrolled off the top come back — drawn by the plugin over a pane whose own
/// grid never moved.
#[test]
fn copy_mode_shows_a_line_the_shell_has_already_scrolled_away() {
    let (mut nt, _init) = start_shipped("", "bash");
    nt.send("printf 'LINE-%03d\\n' $(seq 1 90)\r");
    nt.wait_for_screen("LINE-090", PATIENCE);
    let scrolled_away = nt.screen_after(Duration::from_millis(400));
    assert!(
        !scrolled_away.contains("LINE-001"),
        "the first line never left the screen, so there is nothing to scroll back to:\n{scrolled_away}"
    );

    nt.send("\x02["); // C-b [
    nt.send("g"); // the oldest line the pane holds
    nt.wait_for_screen("LINE-001", PATIENCE);

    nt.send("q");
    nt.send("echo BACK\r");
    nt.wait_for_screen("BACK", PATIENCE);
    let after = nt.screen_after(Duration::from_millis(400));
    assert!(
        after.contains("LINE-090"),
        "the pane's own view moved while the plugin was drawing over it:\n{after}"
    );
}

/// A cursor walked past the top of the screen takes the view with it, one line at a time.
/// The cursor stays on the top row while it does, so the overlay never changes and only the
/// view does — which is the frame that used to be skipped.
#[test]
fn copy_mode_scrolls_as_the_cursor_walks_off_the_top() {
    let (mut nt, _init) = start_shipped("", "bash");
    nt.send("printf 'LINE-%03d\\n' $(seq 1 90)\r");
    nt.wait_for_screen("LINE-090", PATIENCE);
    let before = nt.screen_after(Duration::from_millis(400));
    assert!(!before.contains("LINE-055"), "{before}");

    nt.send("\x02["); // C-b [
    // Onto the top row first, on its own frame, so the keys after it move nothing but the view.
    nt.send(&"k".repeat(30));
    nt.wait_until("the cursor never reached the top row", PATIENCE, |nt| {
        (0..COLS).any(|col| nt.cell_bg(PANE_TOP, col) == Some(COPY_CURSOR))
    });
    nt.send(&"k".repeat(10));
    nt.wait_for_screen("LINE-055", PATIENCE);
}

/// A selection is visible over the pane's real content, and the content is still readable
/// through it: the cells carry the selection's background and the shell's own characters at
/// the same time.
#[test]
fn a_selection_is_visible_over_text_that_is_still_readable() {
    let (mut nt, _init) = start_shipped("", "cat");
    type_into_cat(&mut nt, "ABCDEFGH");
    assert_eq!(nt.cell(PANE_TOP, 1), "B", "the pane is not where expected");

    nt.send("\x02["); // C-b [
    nt.send("kk"); // up from the pane's cursor to the echoed line
    nt.send("v"); // start the selection under the cursor
    nt.send("lll");
    nt.wait_until(
        "the selection never covered three columns",
        PATIENCE,
        |nt| (0..3).all(|col| nt.cell_bg(PANE_TOP, col) == Some(COPY_SELECTION)),
    );

    let screen = nt.screen_after(Duration::ZERO);
    for col in 0..3 {
        assert_eq!(
            nt.cell_bg(PANE_TOP, col),
            Some(COPY_SELECTION),
            "column {col} is not selected:\n{screen}"
        );
    }
    assert_eq!(
        (
            nt.cell(PANE_TOP, 0),
            nt.cell(PANE_TOP, 1),
            nt.cell(PANE_TOP, 2)
        ),
        ("A".to_string(), "B".to_string(), "C".to_string()),
        "the selection painted the text out:\n{screen}"
    );
    assert_eq!(
        nt.cell_bg(PANE_TOP, 4),
        None,
        "the selection ran past the cursor:\n{screen}"
    );
}

/// Yanking says where the text went, which is all it can say: there is no clipboard in this
/// ABI and no way for a plugin to reach one, so the text lands in the plugin's own register.
#[test]
fn yanking_reports_the_register_it_put_the_text_in() {
    let (mut nt, _init) = start_shipped("", "cat");
    type_into_cat(&mut nt, "ABCDEFGH");

    nt.send("\x02["); // C-b [
    nt.send("vlll");
    nt.send("y");
    // A fragment, not the whole sentence: a notification is drawn in a box now and wraps to
    // fit it, so the long form is no longer one run of characters on one row.
    nt.wait_for_screen("yanked 1 line", PATIENCE);
}

/// Off unless the user says otherwise, like the watcher and the sidebar. Proven by
/// `plugin.clock`'s own size report rather than by content, because the footer draws nothing
/// at all while no mode is raised — so the only observable difference between the plugin
/// being absent and being loaded but silent is the row it took from the panes.
#[test]
fn the_shipped_footer_is_off_by_default() {
    let init = harness::runtime(&[("init.lua", "require('plugin.clock')")]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &harness::fixture_layer(), &shipped_layer()],
        "cat",
    );
    // The status bar's row and `plugin.clock`'s own row, and nothing the footer took.
    nt.wait_for_screen("CLOCK 80x22", PATIENCE);
}

/// The other half: turning the default on costs the panes a second row, the same way any
/// other reservation would.
#[test]
fn turning_the_footer_on_reserves_a_second_row() {
    let init = harness::runtime(&[(
        "init.lua",
        "require('neotmux.defaults').footer = true\nrequire('plugin.clock')",
    )]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &harness::fixture_layer(), &shipped_layer()],
        "cat",
    );
    nt.wait_for_screen("CLOCK 80x21", PATIENCE);
}

/// A band a plugin took and then left alone is a strip of bare terminal under everything
/// else, which reads as a hole rather than as an empty bar. The footer reserves its row
/// whether or not a mode is ever raised into it, so it paints it whether or not one is.
#[test]
fn the_footer_paints_the_row_it_reserved_with_no_mode_raised() {
    let init = harness::runtime(&[("init.lua", "require('neotmux.defaults').footer = true")]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");
    nt.wait_for_status("cat", PATIENCE);

    // The harness starts a 24-row terminal and the footer's band is the last row of it. A
    // screen dump would not answer this: it is trimmed of the very blank line in question.
    nt.wait_until(
        "the footer reserved the bottom row and did not paint it",
        PATIENCE,
        |nt| nt.cell_bg(23, 0).is_some(),
    );
}

/// The same mechanism the shipped defaults already rely on — a later assignment in the same
/// `init.lua` wins because the shipped layer's loop reads the table only once, at the end of
/// the whole init chain — turns the footer off again after a config turned it on.
#[test]
fn a_user_config_can_switch_the_footer_off_again() {
    let init = harness::runtime(&[(
        "init.lua",
        "require('neotmux.defaults').footer = true\n\
         require('neotmux.defaults').footer = false\n\
         require('plugin.clock')",
    )]);
    let mut nt = Neotmux::start_layered(
        &[init.path(), &harness::fixture_layer(), &shipped_layer()],
        "cat",
    );
    nt.wait_for_screen("CLOCK 80x22", PATIENCE);
}

/// The armed prefix names itself in the footer, and stops the moment a command fires — `o`
/// with one pane is a harmless one, focusing the only pane there is.
#[test]
fn the_footer_shows_the_armed_prefix_and_clears_when_it_fires() {
    let (mut nt, _init) = start_shipped("require('neotmux.defaults').footer = true", "cat");
    nt.wait_for_status("cat", PATIENCE);

    nt.send("\x02"); // C-b: arm the prefix
    nt.wait_for_screen(" prefix ", PATIENCE);

    nt.send("o"); // any command fires and disarms
    nt.wait_until(
        "the footer kept showing the armed prefix after it fired",
        PATIENCE,
        |nt| {
            let screen = nt.screen_contents();
            !screen.contains(" prefix ")
        },
    );
}

/// Resize mode names itself too, and differently from the armed prefix — the whole reason
/// `plugin.prefix` was asked to raise two names rather than one.
#[test]
fn the_footer_shows_resize_mode_and_clears_when_it_leaves() {
    let (mut nt, _init) = start_shipped("require('neotmux.defaults').footer = true", "cat");
    nt.send("\x02v"); // C-b v: a second pane, so there is a boundary to resize
    nt.wait_for_status(SECOND_PANE, PATIENCE);

    nt.send("\x02r"); // C-b r: into resize mode
    nt.wait_for_screen(" resize ", PATIENCE);

    nt.send("\r"); // anything that is not an adjustment leaves the mode
    nt.wait_until(
        "the footer kept showing resize mode after it was left",
        PATIENCE,
        |nt| {
            let screen = nt.screen_contents();
            !screen.contains(" resize ")
        },
    );
}

/// Copy mode raises its own name too, entirely independently of the prefix's two — proving
/// the footer reads one shared convention rather than something specific to `plugin.prefix`.
#[test]
fn the_footer_shows_copy_mode_and_clears_when_it_leaves() {
    let (mut nt, _init) = start_shipped("require('neotmux.defaults').footer = true", "cat");
    nt.wait_for_status("cat", PATIENCE);

    nt.send("\x02["); // C-b [: enter copy mode
    nt.wait_for_screen(" copy ", PATIENCE);

    nt.send("q"); // leave
    nt.wait_until(
        "the footer kept showing copy mode after it was left",
        PATIENCE,
        |nt| {
            let screen = nt.screen_contents();
            !screen.contains(" copy ")
        },
    );
}

/// The convention, not just the footer: a plugin the footer has never heard of, defined here
/// rather than in the checked-in fixture set, raises its own name and the footer shows it —
/// and stops the moment that plugin lowers it, with no change to `plugin.footer` at all.
#[test]
fn a_third_party_plugin_raising_its_own_mode_shows_in_the_footer() {
    let init = harness::runtime(&[
        (
            "init.lua",
            "require('neotmux.defaults').footer = true\nrequire('plugin.thirdparty')",
        ),
        (
            "lua/plugin/thirdparty.lua",
            r#"
local modes = require("neotmux.mode")
local handle = nil

neotmux.keymap.set("C-g", function()
  if handle then
    handle:lower()
    handle = nil
  else
    handle = modes.raise("thirdparty")
  end
  return true
end)
"#,
        ),
    ]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");
    nt.wait_for_status("cat", PATIENCE);

    nt.send("\x07"); // C-g: the fixture raises its own mode
    nt.wait_for_screen(" thirdparty ", PATIENCE);

    nt.send("\x07"); // C-g again: it lowers it
    nt.wait_until(
        "the footer kept showing a mode after the plugin that raised it lowered it",
        PATIENCE,
        |nt| {
            let screen = nt.screen_contents();
            !screen.contains(" thirdparty ")
        },
    );
    assert!(
        nt.log_text().is_empty(),
        "the third-party mode plugin was reported as a failure:\n{}",
        nt.log_text()
    );
}

/// The answer to what happens when a second mode is raised over a first: the reader sees the
/// second, and when the second is lowered the first is showing again — its own lower call
/// was never asked to touch anything but the entry it was handed.
#[test]
fn a_raised_mode_is_uncovered_when_the_one_over_it_is_lowered() {
    let init = harness::runtime(&[
        (
            "init.lua",
            "require('neotmux.defaults').footer = true\nrequire('plugin.thirdparty')",
        ),
        (
            "lua/plugin/thirdparty.lua",
            r#"
local modes = require("neotmux.mode")
local handle = nil

neotmux.keymap.set("C-g", function()
  if handle then
    handle:lower()
    handle = nil
  else
    handle = modes.raise("thirdparty")
  end
  return true
end)
"#,
        ),
    ]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");
    nt.wait_for_status("cat", PATIENCE);

    nt.send("\x07"); // C-g: thirdparty raises first
    nt.wait_for_screen(" thirdparty ", PATIENCE);

    nt.send("\x02"); // C-b: the prefix arms over it
    nt.wait_for_screen(" prefix ", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(100));
    assert!(
        !screen.contains(" thirdparty "),
        "both names were on screen at once, not just the one raised last:\n{screen}"
    );

    nt.send("o"); // any command fires and disarms the prefix alone
    nt.wait_for_screen(" thirdparty ", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(100));
    assert!(
        !screen.contains(" prefix "),
        "the prefix's own name survived its own lower:\n{screen}"
    );

    nt.send("\x07"); // C-g: thirdparty lowers its own, nothing left to show
    nt.wait_until(
        "something was still raised after both sides lowered their own",
        PATIENCE,
        |nt| {
            let screen = nt.screen_contents();
            !screen.contains(" thirdparty ") && !screen.contains(" prefix ")
        },
    );
}

/// Issue #9, at the layer it broke: a program that positions with HVP (`ESC [ y ; x f`) draws
/// where it meant to. `vt100` implements CUP and drops HVP, so btop — which positions
/// everything with `f` — came out as one run of text with no gaps in it, boxes and all.
///
/// Driven with `printf` rather than with btop: what broke is the emulation, and a test that
/// needs btop installed to say so is a test about this machine.
#[test]
fn a_pane_positioned_with_hvp_draws_where_it_meant_to() {
    let (mut nt, _init) = start("", "/bin/sh");
    nt.wait_for_screen("$", PATIENCE);
    nt.send("printf '\\033[2J\\033[3;5fLEFT\\033[3;40fRIGHT\\033[6;5fBELOW'\r");

    // Each run on its own row, at the column it asked for — the pane is the whole terminal
    // here, so the sequence's own 1-based row and column are the screen's. An unhandled HVP
    // would have put all three together where the cursor happened to be.
    nt.wait_for_cell(2, 4, "L", PATIENCE);
    nt.wait_for_cell(2, 39, "R", PATIENCE);
    nt.wait_for_cell(5, 4, "B", PATIENCE);
}

/// Copy mode keeps the pane's colours. It used to redraw the rows out of the lines it had
/// taken, and a line out of the ABI is text — so a scrolled view was monochrome. The host
/// snapshots the pane at the place in its own history the mode is showing, so what appears is
/// the pane's own cells with everything the child wrote on them.
#[test]
fn copy_mode_keeps_the_colours_the_pane_was_written_in() {
    let (mut nt, _init) = start_shipped("", "/bin/sh");
    nt.wait_for_screen("$", PATIENCE);
    // More lines than the screen holds, each in a colour of its own.
    nt.send(
        "i=1; while [ $i -le 40 ]; do printf '\\033[3%dmline %s\\033[0m\\n' $((i % 8)) $i; \
         i=$((i+1)); done\r",
    );
    nt.wait_for_screen("line 40", PATIENCE);
    let coloured = |nt: &Neotmux| {
        (1..8)
            .filter(|&row| nt.cell_fg_idx(row, 2).is_some())
            .count()
    };
    nt.wait_until("the fixture did not colour its lines", PATIENCE, |nt| {
        coloured(nt) >= 6
    });

    nt.send("\x02["); // C-b [
    let cursor_row = |nt: &Neotmux| {
        (0..ROWS).find(|&row| (0..COLS).any(|col| nt.cell_bg(row, col) == Some(COPY_CURSOR)))
    };
    nt.wait_until("copy mode never drew its cursor", PATIENCE, |nt| {
        cursor_row(nt).is_some()
    });
    let from = cursor_row(&nt).expect("a cursor");
    for _ in 0..12 {
        nt.send("k");
    }
    let to = from.saturating_sub(12).max(PANE_TOP);
    nt.wait_until("the cursor never walked twelve lines up", PATIENCE, |nt| {
        cursor_row(nt) == Some(to)
    });

    let scrolled: Vec<Option<u8>> = (1..8).map(|row| nt.cell_fg_idx(row, 2)).collect();
    assert!(
        scrolled.iter().filter(|fg| fg.is_some()).count() >= 6,
        "the scrolled view came out monochrome: {scrolled:?}"
    );
    nt.send("q");
}

/// A notification says what it says, whatever language it is in. The host drew these itself
/// and cut them to a count of *characters*, so a line of Japanese overflowed the width it was
/// given and lost cells off the end — `ペインが返事を待っています` came out as `ペン返をっい`.
/// It is a plugin's drawing now, cut in columns by the host's own measure.
#[test]
fn a_notification_in_japanese_keeps_its_characters() {
    let init = harness::runtime(&[(
        "init.lua",
        "require('plugin.prefix').commands['Y'] = function()\n\
           neotmux.notify('warn', 'ペインが返事を待っています')\n\
         end\n",
    )]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");
    nt.wait_for_status("cat", PATIENCE);
    nt.send("\x02Y");
    nt.wait_for_screen("ペインが返事を待っています", PATIENCE);
}

/// And it is readable over whatever the pane was printing: the surface it is drawn on is the
/// plugin's own, with a background and a frame, rather than bare text laid over the cells.
#[test]
fn a_notification_is_drawn_on_a_ground_of_its_own() {
    let init = harness::runtime(&[(
        "init.lua",
        "require('plugin.prefix').commands['Y'] = function()\n\
           neotmux.notify('info', 'noticed')\n\
         end\n",
    )]);
    let mut nt = Neotmux::start_layered(&[init.path(), &shipped_layer()], "cat");
    nt.wait_for_status("cat", PATIENCE);
    nt.send("\x02Y");
    nt.wait_for_screen("noticed", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(200));
    let row = screen
        .lines()
        .position(|line| line.contains("noticed"))
        .expect("the notification is on the screen") as u16;
    let col = screen
        .lines()
        .nth(row as usize)
        .and_then(|line| line.find("noticed"))
        .expect("the notification is on the screen") as u16;
    assert!(
        nt.cell_bg(row, col).is_some(),
        "the notification was drawn straight onto the pane's cells:\n{screen}"
    );
}

/// `C-b z`, tmux's zoom: the focused pane takes the whole window, its terminal grows to match,
/// the divider goes, and the bar says so. Pressed again, the window is as it was.
#[test]
fn zoom_gives_the_focused_pane_the_window_and_gives_it_back() {
    let (mut nt, _init) = start_shipped("", "sh");
    nt.send("\x02v"); // C-b v: pane 2 on the right, focused
    nt.wait_for_status("1.2 ", PATIENCE);
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);

    nt.send("\x02z"); // C-b z
    nt.wait_for_status("1.2Z", PATIENCE);
    nt.wait_until("the divider stayed on a zoomed window", PATIENCE, |nt| {
        nt.cell(PANE_TOP, DIVIDER_COL) != VERTICAL_DIVIDER
    });
    nt.send("stty size\r");
    nt.wait_for_screen("23 80", PATIENCE);

    nt.send("\x02z");
    nt.wait_until("the bar kept saying zoomed", PATIENCE, |nt| {
        !nt.status().contains("1.2Z")
    });
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);
    nt.send("clear; stty size\r");
    nt.wait_for_screen("23 39", PATIENCE);
}

/// Moving to a pane the zoom is hiding unzooms, as tmux's does: `C-b o` from a zoomed pane
/// lands on the next one with the window back.
#[test]
fn focusing_another_pane_ends_the_zoom() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v");
    nt.wait_for_status(SECOND_PANE, PATIENCE);
    nt.send("\x02z");
    nt.wait_for_status("1.2Z", PATIENCE);

    nt.send("\x02o"); // C-b o
    nt.wait_for_status(FIRST_PANE, PATIENCE);
    assert!(!nt.status().contains('Z'), "{}", nt.status());
    nt.wait_for_cell(PANE_TOP, DIVIDER_COL, VERTICAL_DIVIDER, PATIENCE);
}

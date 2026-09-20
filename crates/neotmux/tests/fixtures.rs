//! The §4.11 fixture plugin set: real `.lua` files under `tests/fixtures`, loaded through
//! the ordinary runtime-layer resolution and the ordinary `require`, driven over a PTY.

use std::path::{Path, PathBuf};
use std::time::Duration;

mod harness;
use harness::Neotmux;

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
fn remover_removes_twice_without_raising_and_its_callback_never_runs() {
    let (mut nt, _init) = start(
        "require('plugin.clock') require('plugin.remover')",
        "/bin/cat",
    );
    nt.wait_for_screen("CLOCK 80x23", PATIENCE);
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
    assert!(
        !screen.contains("^A"),
        "the prefix key reached the shell:\n{screen}"
    );
    assert!(
        !screen.contains('z'),
        "the command key after the prefix reached the shell:\n{screen}"
    );
    assert!(
        screen.contains('Q'),
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
    nt.settle(Duration::from_millis(500));
    nt.send("\x0f"); // C-o: focus the other pane
    nt.settle(Duration::from_millis(500));
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
    nt.settle(Duration::from_millis(500));

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
    nt.settle(Duration::from_millis(500));
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
    nt.settle(Duration::from_millis(500));

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
    assert!(
        screen.contains("CLOCK 80x23"),
        "the status row went with the offender:\n{screen}"
    );
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
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);
    nt.settle(Duration::from_millis(500));

    nt.send("RIGHT");
    nt.settle(Duration::from_millis(500));
    nt.send("\x02o"); // C-b o
    nt.settle(Duration::from_millis(500));
    nt.send("LEFT");

    nt.wait_for_screen("LEFT", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(500));
    let row = screen.lines().next().unwrap_or_default().to_string();

    let left = row
        .find("LEFT")
        .unwrap_or_else(|| panic!("the pane the prefix walked back to never echoed:\n{screen}"));
    let right = row
        .find("RIGHT")
        .unwrap_or_else(|| panic!("`C-b v` never produced a second pane:\n{screen}"));
    assert!(left < 40 && right >= 40, "{screen}");
    assert!(
        !screen.contains('v') && !screen.contains("^B"),
        "a prefix keystroke reached a shell:\n{screen}"
    );
}

/// Task 4: `C-b -` splits the other way — the two panes are stacked, not side by side.
#[test]
fn the_prefix_splits_horizontally() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02-"); // C-b -
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);
    nt.settle(Duration::from_millis(500));

    nt.send("BELOW");
    nt.settle(Duration::from_millis(500));
    nt.send("\x02o");
    nt.settle(Duration::from_millis(500));
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
    assert!(
        !screen.contains('-'),
        "the command key reached the shell:\n{screen}"
    );
}

/// The shipped surface is 80 columns and, with the status row, 23 usable rows. A gutter of
/// one goes between the panes, so a vertical split leaves the left pane columns 0..39, the
/// divider column 40 and the right pane the rest; a horizontal one leaves the top pane rows
/// 0..10, the divider row 11 and the bottom pane the rest.
const PANE_ROWS: usize = 23;
const PANE_COLS: usize = 80;
const DIVIDER_COL: usize = 40;
const DIVIDER_ROW: usize = 11;
const VERTICAL_DIVIDER: char = '\u{2502}';
const HORIZONTAL_DIVIDER: char = '\u{2500}';

/// The UI a split had no way to show: with the panes tiling the surface exactly, nothing on
/// screen said where one ended. `C-b v` now rules the reserved column between them, and a
/// shell is still live on each side of it.
#[test]
fn a_vertical_split_draws_a_divider_column_between_the_panes() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02v"); // C-b v
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);
    nt.settle(Duration::from_millis(500));

    nt.send("RIGHT");
    nt.settle(Duration::from_millis(500));
    nt.send("\x02o"); // C-b o
    nt.settle(Duration::from_millis(500));
    nt.send("LEFT");
    nt.wait_for_screen("LEFT", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    let rows: Vec<&str> = screen.lines().collect();
    let ruled = rows
        .iter()
        .take(PANE_ROWS)
        .filter(|row| row.chars().nth(DIVIDER_COL) == Some(VERTICAL_DIVIDER))
        .count();
    assert_eq!(
        ruled, PANE_ROWS,
        "no divider column down the boundary between the panes:\n{screen}"
    );

    let top = rows.first().copied().unwrap_or_default();
    let left = top
        .find("LEFT")
        .unwrap_or_else(|| panic!("the left pane never echoed:\n{screen}"));
    let right = top
        .find("RIGHT")
        .unwrap_or_else(|| panic!("the right pane never echoed:\n{screen}"));
    assert!(
        left < DIVIDER_COL && right > DIVIDER_COL,
        "the divider is not between the two shells:\n{screen}"
    );
}

/// The same, the other way up: `C-b -` rules the boundary row, with a shell above and below.
#[test]
fn a_horizontal_split_draws_a_divider_row_between_the_panes() {
    let (mut nt, _init) = start_shipped("", "cat");
    nt.send("\x02-"); // C-b -
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);
    nt.settle(Duration::from_millis(500));

    nt.send("BELOW");
    nt.settle(Duration::from_millis(500));
    nt.send("\x02o"); // C-b o
    nt.settle(Duration::from_millis(500));
    nt.send("ABOVE");
    nt.wait_for_screen("ABOVE", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    let rows: Vec<&str> = screen.lines().collect();
    let rule = rows.get(DIVIDER_ROW).copied().unwrap_or_default();
    assert_eq!(
        rule.chars().filter(|c| *c == HORIZONTAL_DIVIDER).count(),
        PANE_COLS,
        "no divider row across the boundary between the panes:\n{screen}"
    );

    let line_of = |needle: &str| {
        rows.iter()
            .position(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("{needle} never reached a pane:\n{screen}"))
    };
    assert!(
        line_of("ABOVE") < DIVIDER_ROW && line_of("BELOW") > DIVIDER_ROW,
        "the divider is not between the two shells:\n{screen}"
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
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

/// Task 5: the default status line is a shipped plugin, so it is there with no user config
/// at all — and it reports the pane tree, which means it follows a split and the focus.
#[test]
fn the_shipped_status_line_shows_with_no_user_config() {
    let init = harness::runtime(&[("init.lua", "require('plugin.splitter')")]);
    let fixtures = harness::fixture_layer();
    let shipped = shipped_layer();
    let mut nt = Neotmux::start_layered(&[init.path(), &fixtures, &shipped], "cat");

    nt.wait_for_screen("panes 1 focus 1", PATIENCE);
    nt.send("\x02"); // C-b: split, focusing the new pane
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);
    nt.send("\x0f"); // C-o: focus the other one
    nt.wait_for_screen("panes 2 focus 1", PATIENCE);
    assert_eq!(
        nt.cell(23, 0),
        "p",
        "the status line is not in the row it reserved:\n{}",
        nt.screen_after(Duration::ZERO)
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

    nt.wait_for_screen("panes 1 focus 1", PATIENCE);
    nt.send("\x02"); // C-b: splitter splits the width, so the new pane is the right one
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);
    nt.send("R"); // into the right pane's first column, which the divider must not take
    nt.send("\x0f"); // C-o: back to the left pane
    // The focus op is applied at the end of the frame the key was read in, so keys sent
    // before the status catches up would still land in the pane on the right.
    nt.wait_for_screen("panes 2 focus 1", PATIENCE);
    nt.send(FILL);

    nt.wait_for_screen(FILL, PATIENCE);
    assert_eq!(
        nt.cell(0, 39),
        "d",
        "the border ate the last column of the left pane:\n{}",
        nt.screen_after(Duration::ZERO)
    );
    assert_eq!(
        nt.cell(0, 40),
        "\u{2502}",
        "the divider is not in the reserved column:\n{}",
        nt.screen_after(Duration::ZERO)
    );
    assert_eq!(
        nt.cell(0, 41),
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

    nt.wait_for_screen("panes 1 focus 1", PATIENCE);
    nt.send("\x02"); // C-b: split, focusing the new pane on the right
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);
    nt.send("R");
    nt.wait_for_screen("R", PATIENCE);

    let screen = nt.screen_after(Duration::from_millis(500));
    assert!(
        !screen.contains(VERTICAL_DIVIDER),
        "a divider survived the opt-out:\n{screen}"
    );
    assert_eq!(
        nt.cell(0, DIVIDER_COL as u16),
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
    nt.wait_for_screen("panes 1 focus 1", PATIENCE);
    for _ in 0..30 {
        nt.send("ZZZZZZZZ\r");
    }
    nt.settle(Duration::from_millis(500));

    let screen = nt.screen_after(Duration::from_millis(300));
    let rows: Vec<&str> = screen.lines().collect();
    assert!(
        rows[0].contains('Z'),
        "the shell never filled its pane:\n{screen}"
    );
    assert!(
        !rows[23].contains('Z'),
        "the shell wrote into the reserved band:\n{screen}"
    );
    assert!(
        rows[23].starts_with("panes 1 focus 1"),
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
    nt.wait_for_screen("panes 1 focus 1", PATIENCE);
    nt.settle(Duration::from_millis(500));
    let before = resizes(&nt.log_text()).len();

    nt.send("\x12"); // C-r: reserve a second bottom row
    nt.wait_for_screen("BAND", PATIENCE);
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
        screen.contains("panes 1 focus 1"),
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
    assert!(
        !screen.contains("panes "),
        "the shipped status line ignored the opt-out:\n{screen}"
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
        !screen.contains("panes "),
        "a status line survived the deletion:\n{screen}"
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
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
    nt.wait_for_screen("panes 1 focus 1", PATIENCE);

    // Out of the box, with no user Lua at all: the shipped prefix is the only way to reach
    // a second pane, so this is what v0.1's line costs an installed user.
    nt.send("\x02v"); // C-b v
    nt.wait_for_screen("panes 2 focus 2", PATIENCE);

    let log = nt.log_text();
    assert!(
        log.is_empty(),
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
    nt.wait_for_screen("panes 1 focus 1", PATIENCE);
    nt.settle(Duration::from_millis(500));
    let before = resizes(&nt.log_text()).len();

    nt.send("\x02"); // C-b: split, focusing the new pane
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
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
        screen.contains("panes 2 focus 2"),
        "the status row did not survive the split:\n{screen}"
    );
}

/// The same for a focus change, which invalidated the epoch for the same reason: focus is one
/// of the three things it is made of.
#[test]
fn a_focus_change_does_not_resize_every_pane() {
    let (mut nt, _init) = resize_probe();
    nt.wait_for_screen("panes 1 focus 1", PATIENCE);
    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);
    nt.settle(Duration::from_millis(500));
    let before = resizes(&nt.log_text()).len();

    nt.send("\x0f"); // C-o: focus the other pane
    nt.wait_for_log("splitter focus_changed 1", PATIENCE);
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
        screen.contains("panes 2 focus 1"),
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
    nt.settle(Duration::from_millis(700));

    let bytes = nt.bytes_in(Duration::from_secs(1));
    assert_eq!(bytes, 0, "an idle session redrew {bytes} bytes");
}

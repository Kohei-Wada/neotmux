//! The §4.11 fixture plugin set: real `.lua` files under `tests/fixtures`, loaded through
//! the ordinary runtime-layer resolution and the ordinary `require`, driven over a PTY.

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

fn line_with<'a>(log: &'a str, marker: &str) -> &'a str {
    log.lines()
        .find(|line| line.contains(marker))
        .unwrap_or_else(|| panic!("no log line contains {marker:?}:\n{log}"))
}

fn index_of(log: &str, marker: &str) -> usize {
    log.lines()
        .position(|line| line.contains(marker))
        .unwrap_or_else(|| panic!("no log line contains {marker:?}:\n{log}"))
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
fn the_pane_stub_answers_today_and_its_ops_travel_on_the_op_buffer() {
    let (nt, _init) = start("require('plugin.panes')", "/bin/cat");
    let log = nt.wait_for_log("panes list after split=", PATIENCE);

    let listed = line_with(&log, "panes list=");
    assert!(listed.contains("list=1 first=1 current=1"), "{listed}");

    let bad = line_with(&log, "panes bad direction");
    assert!(bad.contains("ok=false"), "{bad}");
    assert!(bad.contains("unknown split direction"), "{bad}");

    let split = line_with(&log, "pane.split('vertical') arrived");
    assert!(split.contains("plugin/panes"), "{split}");
    assert!(split.contains("nothing changed"), "{split}");

    let focus = line_with(&log, "pane.focus(%1) arrived");
    assert!(focus.contains("plugin/panes"), "{focus}");

    assert!(
        line_with(&log, "panes list after split=").contains("after split=1"),
        "split mutated the pane list synchronously, inside the callback"
    );
    // split and focus travel on the same single op buffer as the fixture's own log calls,
    // so they land in the order they were pushed (§4.6), not wherever the binding ran.
    assert!(
        index_of(&log, "panes list=1") < index_of(&log, "pane.split('vertical')")
            && index_of(&log, "pane.split('vertical')") < index_of(&log, "pane.focus(%1)")
            && index_of(&log, "pane.focus(%1)") < index_of(&log, "panes list after split="),
        "the pane ops did not drain in the order they were pushed:\n{log}"
    );
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
    let (mut nt, _init) = start("require('plugin.prefix')", "cat");
    nt.send("\x01"); // C-a, the prefix
    nt.send("z"); // the command key, bound by nobody
    nt.send("Q"); // unrelated, no prefix pending
    nt.send("\r");

    let log = nt.wait_for_log("prefix command z", PATIENCE);
    assert!(line_with(&log, "prefix armed").contains("plugin/prefix"));
    assert!(line_with(&log, "prefix command z").contains("plugin/prefix"));

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

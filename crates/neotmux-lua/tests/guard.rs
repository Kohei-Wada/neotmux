//! `plugin.guard`: the Enter a person presses, held back once when the line it would run
//! matches a rule — and the two states it refuses to have an opinion in.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{
    Epoch, Geometry, LineMode, PaneId, PaneTty, Rect, Size, WindowId, geometry::Window,
};
use std::collections::BTreeMap;

mod support;

use support::Built;

const SIZE: Size = Size { rows: 6, cols: 40 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// A layer of the test's own in front of the shipped one, so `plugin.guard` and
/// `neotmux.guard` are the real files.
fn built(init: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
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
            windows: vec![Window {
                id: WindowId::from(1),
                panes: vec![pane()],
                name: None,
            }],
            window: Some(WindowId::from(1)),
        },
    );
    (dir, Built::new(sub, terminals))
}

/// The layer every test here loads: the plugin on, and one rule in the shared table.
const RULE: &str = "require('neotmux.defaults').guard = true
     require('neotmux.guard').block('rm%s+%-rf', 'rm -rf is blocked')";

/// What the pane is showing, with the cursor left on the last line of it — the line being
/// typed, as the host would publish it.
fn typing(sub: &Built, text: &str) {
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), text.as_bytes());
    tty(
        sub,
        cursor_row(text),
        Some(LineMode {
            canonical: true,
            echo: true,
        }),
        false,
    );
}

fn cursor_row(text: &str) -> u16 {
    u16::try_from(text.matches("\r\n").count()).unwrap()
}

fn tty(sub: &Built, row: u16, line: Option<LineMode>, alternate: bool) {
    sub.publish_pane_tty(
        support::one(),
        BTreeMap::from([(
            pane(),
            PaneTty {
                cursor: (row, 0),
                cursor_visible: true,
                alternate,
                application_cursor: false,
                line,
            },
        )]),
    );
}

#[test]
fn the_first_enter_on_a_line_a_rule_matches_is_held_back() {
    let (_dir, sub) = built(RULE);
    typing(&sub, "$ rm -rf build");
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
}

/// The second one is the person saying they meant it.
#[test]
fn the_second_enter_on_the_same_line_goes_through() {
    let (_dir, sub) = built(RULE);
    typing(&sub, "$ rm -rf build");
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Fallthrough
    );
}

/// Nothing to say about a line no rule names, and nothing consumed.
#[test]
fn a_line_no_rule_names_is_not_held_back() {
    let (_dir, sub) = built(RULE);
    typing(&sub, "$ cargo test");
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Fallthrough
    );
}

/// The state this must never act in: a line being read and not echoed is a password, and the
/// text on the screen is somebody else's.
#[test]
fn a_password_prompt_is_left_alone() {
    let (_dir, sub) = built(RULE);
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), b"rm -rf build");
    tty(
        &sub,
        0,
        Some(LineMode {
            canonical: true,
            echo: false,
        }),
        false,
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Fallthrough
    );
}

/// Enter in vim is not a command. A full-screen program keeps its keys.
#[test]
fn a_full_screen_program_is_left_alone() {
    let (_dir, sub) = built(RULE);
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), b"rm -rf build");
    tty(
        &sub,
        0,
        Some(LineMode {
            canonical: true,
            echo: true,
        }),
        true,
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Fallthrough
    );
}

/// A pane whose tty the host could not read says nothing about how it is reading, and a guard
/// that acts on nothing is a guard acting on a guess.
#[test]
fn a_pane_with_no_line_discipline_is_still_guarded() {
    let (_dir, sub) = built(RULE);
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), b"rm -rf build");
    tty(&sub, 0, None, false);
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
}

/// The rule table is the one the agent-facing half reads, so a rule added for one is a rule
/// for both — that is the whole reason this asks through `verdict`.
#[test]
fn a_function_rule_can_look_at_the_pane_rather_than_the_text() {
    let (_dir, sub) = built(
        "require('neotmux.defaults').guard = true
         local guard = require('neotmux.guard')
         guard.rule(function(what)
           if what.pane == 1 and what.from == 'terminal' then
             return 'this pane is production'
           end
         end)",
    );
    typing(&sub, "$ anything at all");
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
}

/// Off by default: a shipped guard with nobody's rules in it would still be a key nobody
/// asked to have taken.
#[test]
fn the_plugin_is_off_until_it_is_turned_on() {
    let (_dir, sub) = built("require('neotmux.guard').block('rm%s+%-rf', 'blocked')");
    typing(&sub, "$ rm -rf build");
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Fallthrough
    );
}

/// The collision this file exists to prevent, found by running it: one table, two readers. A
/// layer that narrows its own rules to agents must not silence the rules a person's Enter is
/// checked against — `only` sets a default for what comes next, it does not filter the table.
#[test]
fn a_rule_for_agents_does_not_silence_the_rules_for_the_person() {
    let (_dir, sub) = built(
        "require('neotmux.defaults').guard = true
         local guard = require('neotmux.guard')
         guard.block('rm%s+%-rf', 'rm -rf is blocked')
         guard.only('tool')
         guard.block('anything', 'agents may not say anything')",
    );
    typing(&sub, "$ rm -rf build");
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
}

/// And the narrowing still narrows: a rule added after `only('tool')` is not one a person's
/// own Enter answers to.
#[test]
fn a_rule_narrowed_to_agents_is_not_asked_of_a_person() {
    let (_dir, sub) = built(
        "require('neotmux.defaults').guard = true
         local guard = require('neotmux.guard')
         guard.only('tool')
         guard.block('rm%s+%-rf', 'rm -rf is blocked')",
    );
    typing(&sub, "$ rm -rf build");
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Fallthrough
    );
}

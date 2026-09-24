//! `plugin.palette`: `C-b :` opens every command there is, fuzzy-filtered as you type, run
//! with Enter — a composition on `neotmux.list` (see `tests/list.rs`) and `neotmux.command`
//! (see `tests/commands.rs`).

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size, WindowId, geometry::Window};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 20, cols: 60 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// Three commands of the test's own, registered ahead of the real `plugin.prefix` and
/// `plugin.palette` — one that only logs, one that logs under a name `alpha` does not share a
/// prefix with, and one that answers a line for the palette to notify.
const COMMANDS: &str = "
neotmux.command.add('alpha', function() neotmux.log.info('ran alpha') end, { desc = 'the first one' })
neotmux.command.add('bravo', function() neotmux.log.info('ran bravo') end, { desc = 'the second one' })
neotmux.command.add('cascade', function() return 'a line back' end, { desc = 'answers a line' })
require('plugin.prefix')
require('plugin.palette')
";

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

/// `C-b :`, the real key sequence — there is no stand-in key here, the way `tests/picker.rs`
/// shows driving `plugin.prefix` for real.
fn open(sub: &LuaSubsystem) {
    assert_eq!(
        sub.dispatch_key(support::one(), "C-b"),
        KeyOutcome::Consumed
    );
    assert_eq!(sub.dispatch_key(support::one(), ":"), KeyOutcome::Consumed);
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        sub.dispatch_key(support::one(), spec);
    }
}

fn frame(sub: &LuaSubsystem) -> Vec<DrawCmd> {
    sub.dispatch_tick(support::one());
    sub.frame_output()
}

fn texts(cmds: &[DrawCmd]) -> Vec<String> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn notified(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// Every registered command is drawn, its description with it.
#[test]
fn the_palette_lists_every_registered_command() {
    let (_dir, sub) = built(COMMANDS);
    open(&sub);

    let said = texts(&frame(&sub));
    assert!(said.iter().any(|t| t == "alpha"), "{said:?}");
    assert!(said.iter().any(|t| t.contains("the first one")), "{said:?}");
    assert!(said.iter().any(|t| t == "bravo"), "{said:?}");
    assert!(said.iter().any(|t| t == "cascade"), "{said:?}");
}

/// Typing narrows the list to names whose characters the query matches in order — `br` is in
/// `bravo` in order, and in neither `alpha` nor `cascade`.
#[test]
fn typing_narrows_the_list_to_what_matches_in_order() {
    let (_dir, sub) = built(COMMANDS);
    open(&sub);
    keys(&sub, &["b", "r"]);

    let said = texts(&frame(&sub));
    assert!(said.iter().any(|t| t == "bravo"), "{said:?}");
    assert!(!said.iter().any(|t| t == "alpha"), "{said:?}");
    assert!(!said.iter().any(|t| t == "cascade"), "{said:?}");
}

/// `Enter` runs the command under the cursor and closes the palette.
#[test]
fn enter_runs_the_command_under_the_cursor() {
    let (_dir, sub) = built(COMMANDS);
    open(&sub);
    keys(&sub, &["a", "l", "p", "h", "a"]); // narrows to exactly "alpha"

    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
    assert!(frame(&sub).is_empty(), "the palette did not close on Enter");
    let lines = logged(&sub);
    assert!(lines.iter().any(|l| l == "ran alpha"), "{lines:?}");
}

/// A command that answers a line has it raised with `neotmux.notify`.
#[test]
fn a_command_that_answers_lines_has_them_notified() {
    let (_dir, sub) = built(COMMANDS);
    open(&sub);
    keys(&sub, &["c", "a", "s", "c", "a", "d", "e"]); // narrows to exactly "cascade"

    sub.dispatch_key(support::one(), "Enter");
    let lines = notified(&sub);
    assert!(lines.iter().any(|l| l == "a line back"), "{lines:?}");
}

/// `plugin.prefix`'s own verbs are registered commands, so a fresh install has something for
/// the palette to list.
#[test]
fn the_prefix_verbs_are_registered_as_commands() {
    let (_dir, sub) = built("require('plugin.prefix')");

    let names: Vec<String> = sub.commands().into_iter().map(|c| c.name).collect();
    assert!(names.iter().any(|n| n == "split-vertical"), "{names:?}");
    assert!(names.iter().any(|n| n == "detach"), "{names:?}");
}

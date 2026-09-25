//! `plugin.palette`: `C-b :` opens every command there is, fuzzy-filtered as you type, run
//! with Enter — a composition on `neotmux.list` (see `tests/list.rs`) and `neotmux.command`
//! (see `tests/commands.rs`).

use std::collections::BTreeMap;
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
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

/// One string per drawn row, in row order. A name is drawn in several runs now — the letters
/// the query matched are lit, so they are runs of their own — and a row is what a person
/// reads, so a row is what these assert on.
fn texts(cmds: &[DrawCmd]) -> Vec<String> {
    let mut rows: BTreeMap<u16, Vec<(u16, String)>> = BTreeMap::new();
    for cmd in cmds {
        if let DrawCmd::Text { x, y, text, .. } = cmd {
            rows.entry(*y).or_default().push((*x, text.clone()));
        }
    }
    rows.into_values()
        .map(|mut runs| {
            runs.sort_by_key(|(x, _)| *x);
            let row: String = runs.into_iter().map(|(_, text)| text).collect();
            // The frame is drawn into the same rows as the content it holds; a reader does not
            // count it as part of the line and neither do these tests.
            row.trim_matches(|c| c == '│').to_string()
        })
        .collect()
}

/// The name each item row starts with. A row is the name, then its description, then the
/// source pushed right — so the name is what it begins with, and that is what the tests about
/// which items are shown and in what order are about.
fn names(rows: &[String], of: &[&str]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| {
            of.iter()
                .find(|name| row.starts_with(*name))
                .map(|name| (*name).to_string())
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
    assert!(said.iter().any(|t| t.starts_with("alpha")), "{said:?}");
    assert!(said.iter().any(|t| t.contains("the first one")), "{said:?}");
    assert!(said.iter().any(|t| t.starts_with("bravo")), "{said:?}");
    assert!(said.iter().any(|t| t.starts_with("cascade")), "{said:?}");
}

/// Typing narrows the list to names whose characters the query matches in order — `br` is in
/// `bravo` in order, and in neither `alpha` nor `cascade`.
#[test]
fn typing_narrows_the_list_to_what_matches_in_order() {
    let (_dir, sub) = built(COMMANDS);
    open(&sub);
    keys(&sub, &["b", "r"]);

    let said = texts(&frame(&sub));
    assert!(said.iter().any(|t| t.starts_with("bravo")), "{said:?}");
    assert!(!said.iter().any(|t| t.starts_with("alpha")), "{said:?}");
    assert!(!said.iter().any(|t| t.starts_with("cascade")), "{said:?}");
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

/// The names the ordering and gap tests below register, kept apart from `COMMANDS` so each
/// test's rows are exactly the ones it names — nothing to filter out but each other.
const GAP: &str = "
neotmux.command.add('bravo', function() end)
neotmux.command.add('vibrant', function() end)
require('plugin.prefix')
require('plugin.palette')
";

/// `bv` is nowhere contiguous in `bravo` — the query only matches because the matcher is
/// fuzzy, not a substring search — and `vibrant` holds the same two letters in the wrong
/// order, so it is not matched at all. Both are what tells the two apart: a substring filter
/// would fail `bravo` too, and an order-blind one would pass `vibrant`.
#[test]
fn a_query_with_a_gap_matches_in_order_and_refuses_the_wrong_order() {
    let (_dir, sub) = built(GAP);
    open(&sub);
    keys(&sub, &["b", "v"]);

    let said = texts(&frame(&sub));
    assert!(said.iter().any(|t| t.starts_with("bravo")), "{said:?}");
    assert!(!said.iter().any(|t| t.starts_with("vibrant")), "{said:?}");
}

const RANKED: &str = "
neotmux.command.add('abxyz', function() end)
neotmux.command.add('aqqqqb', function() end)
neotmux.command.add('Zzzab', function() end)
require('plugin.prefix')
require('plugin.palette')
";

/// Three names, ranked by the query `ab` three different ways: `abxyz` matches at the front
/// with nothing between the two letters (first 1, spread 1); `aqqqqb` matches at the same
/// front but with four letters between them (first 1, spread 5); `Zzzab` matches four
/// characters in (first 4). First position outranks spread, so the expected order is `abxyz`,
/// `aqqqqb`, `Zzzab` — read off in draw order, not with `any()`.
///
/// `Zzzab`'s leading capital is not decorative: `neotmux.command.list()` answers in name
/// order out of a `BTreeMap`, byte-wise, and a capital sorts before every lowercase letter — so
/// this set's *registered* order is `Zzzab`, `abxyz`, `aqqqqb`, nothing like the ranked order
/// above. A scorer that forgot to rank, and merely kept what `items()` was handed, would answer
/// that registered order here and fail this assertion instead of coincidentally passing it.
#[test]
fn typing_ranks_by_first_match_then_by_how_tightly_the_rest_followed() {
    let (_dir, sub) = built(RANKED);
    open(&sub);
    keys(&sub, &["a", "b"]);

    let order = names(&texts(&frame(&sub)), &["abxyz", "aqqqqb", "Zzzab"]);
    assert_eq!(order, vec!["abxyz", "aqqqqb", "Zzzab"]);
}

const TIED: &str = "
neotmux.command.add('bee', function() end)
neotmux.command.add('cee', function() end)
require('plugin.prefix')
require('plugin.palette')
";

/// `bee` and `cee` score identically against `e` — the match lands on the same character, one
/// deep, in both — so the tie is broken by name, not by whichever was registered first.
#[test]
fn a_tied_score_falls_back_to_name_order() {
    let (_dir, sub) = built(TIED);
    open(&sub);
    keys(&sub, &["e"]);

    let order = names(&texts(&frame(&sub)), &["bee", "cee"]);
    assert_eq!(order, vec!["bee", "cee"]);
}

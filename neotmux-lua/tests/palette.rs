//! `plugin.palette`: `C-b :` opens every command there is, fuzzy-filtered as you type, run
//! with Enter — a composition on `neotmux.list` (see `tests/list.rs`) and `neotmux.command`
//! (see `tests/commands.rs`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size, WindowId, geom::Window};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 20, cols: 60 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

/// Three commands of the test's own, registered ahead of the real `plugin.tmux.prefix` and
/// `plugin.palette` — one that only logs, one that logs under a name `alpha` does not share a
/// prefix with, and one that answers a line for the palette to notify.
const COMMANDS: &str = "
neotmux.command.add('alpha', function() neotmux.log.info('ran alpha') end, { desc = 'the first one' })
neotmux.command.add('bravo', function() neotmux.log.info('ran bravo') end, { desc = 'the second one' })
neotmux.command.add('cascade', function() return 'a line back' end, { desc = 'answers a line' })
require('plugin.tmux.prefix')
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
            zoomed: None,
        },
    );
    (dir, Built::new(sub, terminals))
}

/// `C-b :`, the real key sequence — there is no stand-in key here, the way `tests/picker.rs`
/// shows driving `plugin.tmux.prefix` for real.
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

/// `plugin.tmux.prefix`'s own verbs are registered commands, so a fresh install has something for
/// the palette to list.
#[test]
fn the_prefix_verbs_are_registered_as_commands() {
    let (_dir, sub) = built("require('plugin.tmux.prefix')");

    let names: Vec<String> = sub.commands().into_iter().map(|c| c.name).collect();
    assert!(names.iter().any(|n| n == "split-vertical"), "{names:?}");
    assert!(names.iter().any(|n| n == "detach"), "{names:?}");
}

/// The names the ordering and gap tests below register, kept apart from `COMMANDS` so each
/// test's rows are exactly the ones it names — nothing to filter out but each other.
const GAP: &str = "
neotmux.command.add('bravo', function() end)
neotmux.command.add('vibrant', function() end)
require('plugin.tmux.prefix')
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
require('plugin.tmux.prefix')
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
require('plugin.tmux.prefix')
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

/// A second session beside the one the tests drive, named so the palette's `@` has two to show.
fn with_home(sub: &Built) {
    sub.name_session(support::one(), "work");
    let home = neotmux_proto::SessionId::from(2);
    sub.name_session(home, "home");
    let pane = PaneId::from(2);
    sub.begin_frame(
        home,
        Epoch::new(SIZE, vec![pane], Some(pane)),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane, Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane),
            windows: vec![Window {
                id: WindowId::from(2),
                panes: vec![pane],
                name: None,
            }],
            window: Some(WindowId::from(2)),
            zoomed: None,
        },
    );
}

fn switches(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::host::Op::SessionSwitch { name } => Some(name),
            _ => None,
        })
        .collect()
}

/// `@` is the sessions: every one there is, and Enter goes to the one under the cursor.
#[test]
fn an_at_sign_lists_the_sessions_and_enter_goes_to_one() {
    let (_dir, sub) = built("");
    with_home(&sub);
    open(&sub);
    keys(&sub, &["@", "h"]);
    let rows = texts(&frame(&sub));
    assert!(rows.iter().any(|row| row.starts_with("home")), "{rows:#?}");
    assert!(!rows.iter().any(|row| row.starts_with("work")), "{rows:#?}");
    keys(&sub, &["Enter"]);
    assert_eq!(switches(&sub), vec!["home".to_string()]);
}

/// Without a prefix it is still the commands, as it always was.
#[test]
fn no_prefix_is_still_the_commands() {
    let (_dir, sub) = built("neotmux.command.add('zebra', function() end)");
    with_home(&sub);
    open(&sub);
    keys(&sub, &["z", "e"]);
    let rows = texts(&frame(&sub));
    assert!(rows.iter().any(|row| row.starts_with("zebra")), "{rows:#?}");
    assert!(!rows.iter().any(|row| row.starts_with("home")), "{rows:#?}");
}

/// `C-b g` opens it on `#`, the agents — herdr's goto: every agent, and Enter focuses its pane.
#[test]
fn c_b_g_opens_on_the_agents_and_enter_focuses_the_agents_pane() {
    let (_dir, sub) = built(
        "require('neotmux.detect').agents = { fake = {\n\
           { id = 'busy', state = 'working', priority = 1,\n\
             match = function(v) return v.lines[#v.lines] == 'BUSY' end },\n\
         } }",
    );
    sub.name_session(support::one(), "work");
    sub.terminals().open(pane(), SIZE, 0);
    sub.terminals().feed(pane(), b"BUSY");
    assert_eq!(
        sub.dispatch_key(support::one(), "C-b"),
        KeyOutcome::Consumed
    );
    assert_eq!(sub.dispatch_key(support::one(), "g"), KeyOutcome::Consumed);
    let rows = texts(&frame(&sub));
    assert!(
        rows.iter()
            .any(|row| row.contains("work") && row.contains("fake")),
        "{rows:#?}"
    );
    keys(&sub, &["Enter"]);
    let focused: Vec<PaneId> = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::host::Op::PaneFocus { pane } => Some(pane),
            _ => None,
        })
        .collect();
    assert_eq!(focused, vec![pane()]);
}

/// A provider of a plugin's own, and one that raises: the second is left out and said once.
#[test]
fn a_provider_can_be_added_and_one_that_raises_is_left_out() {
    let (_dir, sub) = built(
        "local palette = require('plugin.palette')\n\
         table.insert(palette.providers, 1, { prefix = '%', name = 'fruit',\n\
           items = function() return { { name = 'apple' }, { name = 'pear' } } end,\n\
           run = function(item) neotmux.log.info('ate ' .. item.name) end })\n\
         table.insert(palette.providers, 1, { prefix = '!', name = 'broken',\n\
           items = function() error('boom') end, run = function() end })",
    );
    open(&sub);
    keys(&sub, &["%", "p"]);
    let rows = texts(&frame(&sub));
    assert!(rows.iter().any(|row| row.starts_with("pear")), "{rows:#?}");
    keys(&sub, &["Enter"]);
    assert!(logged(&sub).iter().any(|line| line == "ate pear"));
    open(&sub);
    keys(&sub, &["!"]);
    frame(&sub);
    keys(&sub, &["x"]);
    frame(&sub);
    let said: Vec<String> = notified(&sub)
        .into_iter()
        .filter(|t| t.contains("broken"))
        .collect();
    assert_eq!(said.len(), 1, "{said:?}");
}

const SESSIONS: &str =
    "require('plugin.tmux.prefix')\nrequire('plugin.tmux.sessions')\nrequire('plugin.palette')\n";

fn session_ops(sub: &Built) -> Vec<Op> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .map(|(_, op)| op)
        .filter(|op| {
            matches!(
                op,
                Op::SessionCreate { .. } | Op::SessionSwitch { .. } | Op::SessionRename { .. }
            )
        })
        .collect()
}

/// `session-new` from the palette asks for a name, then makes the session and goes there, as
/// the picker's `c` does.
#[test]
fn a_session_can_be_made_from_the_palette() {
    let (_dir, sub) = built(SESSIONS);
    sub.name_session(support::one(), "work");
    open(&sub);
    keys(
        &sub,
        &[
            "s", "e", "s", "s", "i", "o", "n", "-", "n", "e", "w", "Enter",
        ],
    );
    frame(&sub);
    keys(&sub, &["h", "o", "m", "e", "Enter"]);
    assert_eq!(
        session_ops(&sub),
        vec![
            Op::SessionCreate {
                name: "home".into()
            },
            Op::SessionSwitch {
                name: "home".into()
            },
        ]
    );
}

/// `session-rename` asks for the new name of the session you are in, starting from the old one.
#[test]
fn the_session_you_are_in_can_be_renamed_from_the_palette() {
    let (_dir, sub) = built(SESSIONS);
    sub.name_session(support::one(), "work");
    open(&sub);
    keys(
        &sub,
        &[
            "s", "e", "s", "s", "i", "o", "n", "-", "r", "e", "n", "Enter",
        ],
    );
    frame(&sub);
    keys(
        &sub,
        &[
            "Backspace",
            "Backspace",
            "Backspace",
            "Backspace",
            "h",
            "o",
            "m",
            "e",
            "Enter",
        ],
    );
    assert_eq!(
        session_ops(&sub),
        vec![Op::SessionRename {
            from: "work".into(),
            to: "home".into()
        }]
    );
}

//! `neotmux.hints`: two keys pick anything on the screen. The harness is `copy.rs`'s — a real
//! pane fed through `support::panes()`, keys driven with `dispatch_key` — with an `H` bound in
//! the test's own `init.lua` to `hints.open(...)`, since nothing in the shipped layer opens a
//! hint session on its own.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

/// Wide enough for a url and a handful of ip addresses on one row, tall enough to hold more
/// targets than `neotmux.hints.keys` has letters for.
const SIZE: Size = Size { rows: 16, cols: 40 };

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

fn pane() -> PaneId {
    PaneId::from(1)
}

/// A key that opens a hint session with whatever `opts` the test wants, and reports what
/// `done` and `cancel` were given. Nothing in the shipped layer binds a key to
/// `neotmux.hints.open` on its own — that call is always the caller's — so the test binds one
/// itself, the same way `copy.rs` drives copy mode through the prefix it ships with.
fn trigger(opts: &str) -> String {
    format!(
        "local hints = require('neotmux.hints')\n\
         neotmux.on_key(function(key)\n\
           if key == 'H' then\n\
             hints.open({opts})\n\
             return true\n\
           end\n\
           if key == 'F' then\n\
             neotmux.log.info('frozen=' .. tostring(neotmux.pane.frozen(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    )
}

/// The default trigger: built-in finders, and a `done`/`cancel` that each log one line a test
/// can read back.
fn default_trigger() -> String {
    trigger(
        "{\n\
           done = function(target)\n\
             neotmux.log.info('done pane=' .. target.pane .. ' line=' .. target.line\n\
               .. ' col=' .. target.col .. ' text=[' .. target.text .. ']')\n\
           end,\n\
           cancel = function()\n\
             neotmux.log.info('cancelled')\n\
           end,\n\
         }",
    )
}

/// The shipped runtime with every plugin off, so every draw command and every log line in a
/// frame belongs to the hint session and not to the status bar, the prefix or anything else.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'sync' }}) do\n\
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

fn feed(sub: &Built, text: &[u8]) {
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), text);
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        sub.dispatch_key(support::one(), spec);
    }
}

fn open(sub: &LuaSubsystem) {
    keys(sub, &["H"]);
}

fn frame(sub: &LuaSubsystem) -> Vec<DrawCmd> {
    sub.dispatch_tick(support::one());
    sub.frame_output()
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// The labels a frame drew: `pane: None` is what `ctx:text(nil, ...)` records, and nothing
/// else in this harness ever draws on the surface.
fn labels(cmds: &[DrawCmd]) -> Vec<(u16, u16, String)> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text {
                pane: None,
                x,
                y,
                text,
                ..
            } => Some((*x, *y, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_url_on_the_screen_gets_a_label() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"see http://example.com now");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");
    // "see " is four columns wide, ascii, so the url starts at column 4.
    assert_eq!(
        marks[0].0, 4,
        "the label did not land on the url: {marks:?}"
    );
    assert_eq!(marks[0].1, 0);
    assert_eq!(
        marks[0].2.chars().count(),
        1,
        "one target gets a one-character label"
    );
}

#[test]
fn typing_a_label_answers_with_the_target_under_it() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"see http://example.com now");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");
    let label = marks[0].2.clone();

    keys(&sub, &[&label]);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line == "done pane=1 line=0 col=4 text=[http://example.com]"),
        "{said:?}"
    );
}

#[test]
fn two_targets_get_two_different_labels() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"http://a.example one\r\nhttp://b.example two");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 2, "{marks:?}");
    assert_ne!(
        marks[0].2, marks[1].2,
        "the two targets got the same label: {marks:?}"
    );
    for mark in &marks {
        assert_eq!(mark.2.chars().count(), 1, "{marks:?}");
    }
}

#[test]
fn more_targets_than_keys_get_two_character_labels_that_are_not_prefixes() {
    let (_dir, sub) = built(&trigger(
        "{\n\
           targets = { 'ip' },\n\
           done = function(target)\n\
             neotmux.log.info('done text=[' .. target.text .. ']')\n\
           end,\n\
         }",
    ));
    let mut text = String::new();
    for n in 0..12 {
        text.push_str(&format!("10.0.0.{n}\r\n"));
    }
    feed(&sub, text.as_bytes());
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 12, "{marks:?}");

    let mut seen = std::collections::HashSet::new();
    for mark in &marks {
        assert_eq!(
            mark.2.chars().count(),
            2,
            "more targets than keys did not get two-character labels: {marks:?}"
        );
        assert!(seen.insert(mark.2.clone()), "duplicate label: {marks:?}");
    }
    for a in &marks {
        for b in &marks {
            if a.2 != b.2 {
                assert!(!b.2.starts_with(&a.2), "'{}' is a prefix of '{}'", a.2, b.2);
            }
        }
    }

    // Typing one all the way through answers with the target under it, proving the label
    // picked out is not merely well-formed but actually reachable.
    let label = marks[0].2.clone();
    let first = &label[0..1];
    let second = &label[1..2];
    keys(&sub, &[first, second]);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line.starts_with("done text=[10.0.0.")),
        "{said:?}"
    );
}

#[test]
fn esc_leaves_without_answering() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"http://example.com");
    open(&sub);
    assert!(!labels(&frame(&sub)).is_empty());

    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "cancelled"), "{said:?}");
    assert!(
        !said.iter().any(|line| line.starts_with("done ")),
        "esc answered as though a target had been picked: {said:?}"
    );
    assert!(
        labels(&frame(&sub)).is_empty(),
        "labels were still drawn after Esc"
    );

    // The session is gone, not merely quiet: a key it would have claimed now falls through.
    assert_eq!(
        sub.dispatch_key(support::one(), "a"),
        KeyOutcome::Fallthrough
    );
}

#[test]
fn a_label_lands_on_the_column_the_target_starts_at_even_with_wide_characters() {
    let (_dir, sub) = built(&default_trigger());
    // "あ" is two columns, then a one-column space: the url starts at column 3. Its first byte
    // is at byte offset 4 (three bytes for "あ" plus the space) — a module counting bytes
    // instead of columns would put the label there instead.
    feed(&sub, "あ http://x".as_bytes());
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");
    assert_eq!(
        marks[0].0, 3,
        "the label landed on a byte offset, not a column: {marks:?}"
    );
    assert_eq!(marks[0].1, 0);
}

#[test]
fn the_panes_it_labelled_are_frozen_while_it_is_up_and_thawed_after() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"http://example.com");

    keys(&sub, &["F"]);
    assert!(
        logged(&sub).iter().any(|line| line == "frozen=false"),
        "the pane started frozen"
    );

    open(&sub);
    keys(&sub, &["F"]);
    assert!(
        logged(&sub).iter().any(|line| line == "frozen=true"),
        "hints did not freeze the pane it labelled"
    );

    keys(&sub, &["Esc"]);
    keys(&sub, &["F"]);
    assert!(
        logged(&sub).iter().any(|line| line == "frozen=false"),
        "hints did not thaw the pane on the way out"
    );
}

#[test]
fn a_caller_supplied_find_replaces_the_built_in_ones() {
    let (_dir, sub) = built(&trigger(
        "{\n\
           find = function(pane, lines)\n\
             return { { line = 0, col = 2, text = 'CUSTOM' } }\n\
           end,\n\
           done = function(target)\n\
             neotmux.log.info('done text=[' .. target.text .. ']')\n\
           end,\n\
         }",
    ));
    // A real url is on the line too: if the built-in finders were still running, this would
    // label two targets instead of the one `find` returned.
    feed(&sub, b"see http://example.com now");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(
        marks.len(),
        1,
        "the built-in finders ran alongside `find`: {marks:?}"
    );
    assert_eq!(marks[0].0, 2);
    assert_eq!(marks[0].1, 0);

    let label = marks[0].2.clone();
    keys(&sub, &[&label]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "done text=[CUSTOM]"),
        "{said:?}"
    );
}

//! `neotmux.hints`: two keys pick anything on the screen. The harness is `copy.rs`'s — a real
//! pane fed through `support::panes()`, keys driven with `dispatch_key` — with an `H` bound in
//! the test's own `init.lua` to `hints.open(...)`, since nothing in the shipped layer opens a
//! hint session on its own.
//!
//! `plugin.hints` is the other half, below: what `C-b f`, `hint-send` and `hint-open` do with a
//! target once `neotmux.hints` has picked one, driven through the shipped prefix rather than
//! a test-bound key, since that binding is the plugin's own.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neotmux_lua::host::Clipboard;
use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

/// Wide enough for a url and a handful of ip addresses on one row, tall enough to hold more
/// targets than `neotmux.hints.keys` has letters for.
const SIZE: Size = Size { rows: 16, cols: 40 };

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
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
           if key == 'Z' then\n\
             neotmux.pane.freeze(1)\n\
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

/// A trigger whose `done` always raises, for the close path that goes through `fail`.
fn raising_trigger() -> String {
    trigger(
        "{\n\
           done = function(target)\n\
             error('boom')\n\
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

/// A path and a hash can both be true of the same stretch of a line — `hash` reads
/// `/home/user/1234567890abcdef`'s last segment as a hex run on its own — but a person picking
/// between them should not have to. The longer span, the whole path, keeps the label.
#[test]
fn a_target_inside_another_targets_span_does_not_get_its_own_label() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"look at /home/user/1234567890abcdef amount");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(
        marks.len(),
        1,
        "the path and the hash inside it both got labelled: {marks:?}"
    );
    // "look at " is eight columns, ascii, so the path starts at column 8.
    assert_eq!(marks[0].0, 8);
}

/// The same overlap between `url` and `ip`: `192.168.1.1:8080` inside
/// `http://192.168.1.1:8080/status` is a whole url to one finder and an ip to the other.
#[test]
fn a_url_and_the_ip_address_inside_it_do_not_both_get_labelled() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"curl http://192.168.1.1:8080/status");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(
        marks.len(),
        1,
        "the url and the ip inside it both got labelled: {marks:?}"
    );
    // "curl " is five columns, so the url starts at column 5.
    assert_eq!(marks[0].0, 5);
}

/// `neotmux.pane.freeze` answers `true` for any pane that exists, frozen already or not — it
/// is `neotmux.pane.frozen` that says which. A module that branches on `freeze`'s own answer
/// marks a pane copy mode (or anything else) had already frozen as one it froze itself, and
/// then thaws it on the way out — exactly the bug ruling 1 was written against. Covers all
/// three ways a session ends: a label typed, Esc, and a raising `done` that reaches `fail`.
#[test]
fn a_pane_frozen_before_hints_opened_stays_frozen_after_hints_closes_any_way() {
    // Closed by typing a label.
    {
        let (_dir, sub) = built(&default_trigger());
        feed(&sub, b"http://example.com");
        keys(&sub, &["Z"]);
        open(&sub);
        let marks = labels(&frame(&sub));
        assert_eq!(marks.len(), 1, "{marks:?}");
        keys(&sub, &[&marks[0].2]);
        keys(&sub, &["F"]);
        assert!(
            logged(&sub).iter().any(|line| line == "frozen=true"),
            "hints thawed a pane it did not freeze, picking a label"
        );
    }

    // Closed by Esc.
    {
        let (_dir, sub) = built(&default_trigger());
        feed(&sub, b"http://example.com");
        keys(&sub, &["Z"]);
        open(&sub);
        keys(&sub, &["Esc"]);
        keys(&sub, &["F"]);
        assert!(
            logged(&sub).iter().any(|line| line == "frozen=true"),
            "hints thawed a pane it did not freeze, on Esc"
        );
    }

    // Closed through `fail`, by a `done` that raises.
    {
        let (_dir, sub) = built(&raising_trigger());
        feed(&sub, b"http://example.com");
        keys(&sub, &["Z"]);
        open(&sub);
        let marks = labels(&frame(&sub));
        assert_eq!(marks.len(), 1, "{marks:?}");
        keys(&sub, &[&marks[0].2]);
        let said = logged(&sub);
        assert!(
            said.iter()
                .any(|line| line.starts_with("neotmux.hints: done:") && line.contains("boom")),
            "the raising done was not reported: {said:?}"
        );
        keys(&sub, &["F"]);
        assert!(
            logged(&sub).iter().any(|line| line == "frozen=true"),
            "hints thawed a pane it did not freeze, on a raising done"
        );
    }
}

/// A pane can leave the layout while hints are up — closed, or moved to a window nobody is
/// looking at. Neither the tick, which asks `ctx:pane_rect` for a pane that is no longer in
/// it, nor closing the session afterwards, is the pane's problem to have caused.
#[test]
fn a_pane_removed_from_the_layout_mid_session_does_not_stop_hints_closing_cleanly() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"http://example.com");
    open(&sub);
    assert!(!labels(&frame(&sub)).is_empty());

    sub.republish_geometry(
        support::one(),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![],
            focus: None,
            ..Default::default()
        },
    );

    let cmds = frame(&sub);
    assert!(
        labels(&cmds).is_empty(),
        "a label was drawn for a pane no longer in the layout: {cmds:?}"
    );
    assert!(
        !logged(&sub)
            .iter()
            .any(|line| line.starts_with("neotmux.hints:")),
        "the tick over a removed pane raised"
    );

    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "cancelled"), "{said:?}");
    assert!(
        !said.iter().any(|line| line.starts_with("neotmux.hints:")),
        "closing over a removed pane raised: {said:?}"
    );
}

/// Opening a session over one already up cancels it rather than dropping it: the caller of
/// the first one has no other way to learn that its pick is over, and one that yields the
/// keyboard while a pick is up would yield it for good.
#[test]
fn opening_a_second_session_cancels_the_first_rather_than_dropping_it() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"http://example.com");
    open(&sub);
    assert!(!labels(&frame(&sub)).is_empty());

    open(&sub);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "cancelled"),
        "the pre-empted session never reached its caller's cancel: {said:?}"
    );

    // The second session is the one up now, and it answers for itself.
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");
    keys(&sub, &[&marks[0].2]);
    assert!(
        logged(&sub)
            .iter()
            .any(|line| line.starts_with("done pane=1")),
    );
}

/// A session is pre-empted only once there is one to put in its place: a pick cancelled for a
/// screen that turns out to have nothing on it is a pick lost for nothing.
#[test]
fn a_session_that_opens_nothing_leaves_the_one_already_up_alone() {
    let (_dir, sub) = built(&format!(
        "{}\n\
         neotmux.on_key(function(key)\n\
           if key == 'E' then\n\
             require('neotmux.hints').open({{ targets = {{ 'url' }} }})\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
        default_trigger()
    ));
    // A path for the first session to label, and no url for the second one to find. The
    // leading space is not decoration: the path finder anchors on `%f[%S]`, and at the very
    // first column there is no non-space character before it for the frontier to cross.
    feed(&sub, b" /etc/hosts");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");

    keys(&sub, &["E"]);
    assert!(
        !logged(&sub).iter().any(|line| line == "cancelled"),
        "a session that opened nothing cancelled the one that was up"
    );
    assert_eq!(labels(&frame(&sub)), marks, "the labels went away");
    keys(&sub, &[&marks[0].2]);
    assert!(
        logged(&sub)
            .iter()
            .any(|line| line.starts_with("done pane=1")),
        "the session that was up stopped answering"
    );
}

/// A screen with nothing on it to label opens nothing: a session with no targets would take
/// every key and show nothing for them, which reads as a terminal that has stopped answering.
#[test]
fn a_screen_with_nothing_to_label_never_takes_the_keyboard() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"nothing here but words");
    open(&sub);

    assert!(labels(&frame(&sub)).is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough,
        "an empty session took the keyboard"
    );
}

/// A hash is taken whole or not at all. A sha256 is 64 hex characters and a label that
/// answered with the first 40 of them would be a copy of something that is not on the screen —
/// worse, silently, than not labelling it at all.
#[test]
fn a_hash_longer_than_a_git_object_is_answered_whole() {
    let sha256 = "e".repeat(64);
    let (_dir, sub) = built(&default_trigger());
    // Its own pane, wide enough to hold all 64 characters on one row: wrapped across two rows
    // they are two runs on the screen and two targets, which is a different question.
    sub.terminals().open(
        pane(),
        Size {
            rows: SIZE.rows,
            cols: 80,
        },
        100,
    );
    sub.terminals().feed(pane(), sha256.as_bytes());
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");

    keys(&sub, &[&marks[0].2]);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| *line == format!("done pane=1 line=0 col=0 text=[{sha256}]")),
        "{said:?}"
    );
}

/// A caller's own `find` gets the same containment filter the built-in finders get: two hits
/// where one holds the other are one thing on the screen whoever found them.
#[test]
fn a_caller_supplied_find_gets_its_overlapping_hits_filtered_too() {
    let (_dir, sub) = built(&trigger(
        "{\n\
           find = function()\n\
             return {\n\
               { line = 0, col = 0, text = 'alpha' },\n\
               { line = 0, col = 1, text = 'lph' },\n\
             }\n\
           end,\n\
           done = function(target)\n\
             neotmux.log.info('done text=[' .. target.text .. ']')\n\
           end,\n\
         }",
    ));
    feed(&sub, b"alpha");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(
        marks.len(),
        1,
        "the contained hit got its own label: {marks:?}"
    );
    keys(&sub, &[&marks[0].2]);
    assert!(
        logged(&sub).iter().any(|line| line == "done text=[alpha]"),
        "the label landed on the piece rather than the whole"
    );
}

/// A label is held inside its pane: a two-character one on a target that starts in the last
/// column would otherwise be drawn over the border, or over the pane next door.
#[test]
fn a_label_is_clipped_to_the_pane_it_belongs_to() {
    let mut hits = String::new();
    for line in 0..11 {
        hits.push_str(&format!(
            "{{ line = {line}, col = {}, text = 'x' }},\n",
            if line == 10 { SIZE.cols - 1 } else { 0 }
        ));
    }
    let (_dir, sub) = built(&trigger(&format!(
        "{{ find = function() return {{ {hits} }} end }}"
    )));
    feed(&sub, b"anything");
    open(&sub);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 11, "{marks:?}");
    let last = marks
        .iter()
        .find(|mark| mark.1 == 10)
        .unwrap_or_else(|| panic!("{marks:?}"));
    assert_eq!(last.2.chars().count(), 2, "{marks:?}");
    assert_eq!(
        last.0,
        SIZE.cols - 2,
        "a two-character label spilled out of the pane: {marks:?}"
    );
}

// `plugin.hints`: what a person does with a target once one is picked. Its own harness, since
// these three commands are the shipped prefix and the shipped registry, not the bare module
// above — `built` there turns both off to keep a frame free of anything but the module's own
// draw commands, which is exactly what these tests need on.

struct Recording(RefCell<Vec<String>>);

impl Clipboard for Recording {
    fn set(&self, _session: Option<neotmux_proto::SessionId>, text: &str) -> std::io::Result<()> {
        self.0.borrow_mut().push(text.to_string());
        Ok(())
    }
}

/// The shipped runtime with the prefix and `plugin.hints` on and every other plugin off, so a
/// frame's draw commands and a session's ops belong to one of the two and nothing else.
fn built_plugin(extra: &str, clipboard: Rc<dyn Clipboard>) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'copy', 'sessions', 'palette', 'notify',\n\
               'borders', 'sync' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build_with(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        clipboard,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            windows: vec![neotmux_proto::geom::Window {
                id: neotmux_proto::WindowId::from(1),
                panes: vec![pane()],
                name: None,
            }],
            window: Some(neotmux_proto::WindowId::from(1)),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            zoomed: None,
        },
    );
    (dir, Built::new(sub, terminals))
}

/// A key bound in the test's own `init.lua` to `neotmux.command.run(name)`, the same way a
/// test drives a command that ships with no key of its own.
fn run_on(key: &str, name: &str) -> String {
    format!(
        "neotmux.on_key(function(k)\n\
           if k == '{key}' then\n\
             neotmux.command.run('{name}')\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    )
}

#[test]
fn c_b_f_labels_the_screen_and_copies_what_is_picked() {
    let recording = Rc::new(Recording(RefCell::new(Vec::new())));
    let (_dir, sub) = built_plugin("", recording.clone());
    feed(&sub, b"see http://example.com now");

    keys(&sub, &["C-b", "f"]);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");

    let label = marks[0].2.clone();
    keys(&sub, &[&label]);

    assert_eq!(
        *recording.0.borrow(),
        vec!["http://example.com".to_string()],
        "C-b f did not hand the picked target's text to the clipboard"
    );
}

#[test]
fn hint_send_types_the_target_into_the_focused_pane() {
    let (_dir, sub) = built_plugin(
        &run_on("X", "hint-send"),
        Rc::new(neotmux_lua::host::Absent),
    );
    feed(&sub, b"see http://example.com now");

    keys(&sub, &["X"]);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");

    let label = marks[0].2.clone();
    keys(&sub, &[&label]);

    let sends: Vec<_> = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::PaneSend { pane, text } => Some((pane, text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        sends,
        vec![(pane(), "http://example.com".to_string())],
        "hint-send did not type the target's text into the focused pane"
    );
}

/// A target holding a `;` and a backtick — the shipped `url` finder's tail excludes only
/// whitespace and a few closing brackets, so both survive into `target.text` — reaching the
/// pane quoted rather than read as shell syntax, and without a newline: typed and left on the
/// line, not run, since the point of the quoting is a person still looking at it before Enter.
#[test]
fn hint_open_writes_the_opener_quoted_and_does_not_press_enter() {
    let (_dir, sub) = built_plugin(
        &run_on("X", "hint-open"),
        Rc::new(neotmux_lua::host::Absent),
    );
    feed(&sub, b"see http://x/;`id` now");

    keys(&sub, &["X"]);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 1, "{marks:?}");

    let label = marks[0].2.clone();
    keys(&sub, &[&label]);

    let sends: Vec<_> = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::PaneSend { pane, text } => Some((pane, text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        sends,
        vec![(pane(), "xdg-open 'http://x/;`id`'".to_string())],
        "hint-open did not quote the target and leave it unsubmitted"
    );
}

#[test]
fn the_commands_are_registered_for_the_palette() {
    let (_dir, sub) = built_plugin(
        "neotmux.on_tick(function()\n\
           for _, cmd in ipairs(neotmux.command.list()) do\n\
             neotmux.log.info(cmd.name .. ': ' .. tostring(cmd.desc))\n\
           end\n\
         end)\n",
        Rc::new(neotmux_lua::host::Absent),
    );
    sub.dispatch_tick(support::one());
    let said = logged(&sub);
    for name in ["hint-copy", "hint-send", "hint-open"] {
        assert!(
            said.iter()
                .any(|line| { line.starts_with(&format!("{name}: ")) && !line.ends_with(": nil") }),
            "{name} is not registered with a description for the palette: {said:?}"
        );
    }
}

#[test]
fn a_reload_with_a_pick_open_leaves_no_pane_frozen() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"http://example.com");
    open(&sub);
    let one = neotmux_proto::PaneId::from(1);
    assert_eq!(
        sub.terminals().is_frozen(one),
        Some(true),
        "hints did not freeze the pane"
    );
    sub.reload(Some(support::one()));
    assert_eq!(
        sub.terminals().is_frozen(one),
        Some(false),
        "the reload left the pane frozen"
    );
}

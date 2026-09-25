//! `neotmux.selection`: the region between two points, in the coordinates the rest of copy
//! mode uses.
//!
//! The points are **columns**, because that is what `ctx:tint` counts and what the cursor
//! and the highlight are drawn in. Before issue #3 the slice read them as character indices,
//! so on a line holding a wide character the text that was yanked was not the text that was
//! highlighted. These are the cases that say so.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;

mod support;

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

/// `body` runs on every tick, with `sel` bound to the module and `w` to the measurement it
/// takes — the same pair `copy.lua` hands it.
fn built(body: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    let init = format!(
        "local sel = require('neotmux.selection')\n\
         local w = neotmux.width\n\
         neotmux.on_tick(function()\n{body}\nend)\n"
    );
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.dispatch_tick(support::one());
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// `あいうえお` is five characters and ten columns: あ is columns 0-1, い 2-3, う 4-5.
/// Asking for columns 0 to 3 is asking for the first two characters, and nothing else.
#[test]
fn a_slice_of_a_wide_line_is_the_columns_it_was_asked_for() {
    let (_dir, sub) = built(
        "local s = '\u{3042}\u{3044}\u{3046}\u{3048}\u{304a}'\n\
         neotmux.log.info(sel.slice(s, 0, 3, w))\n\
         neotmux.log.info(sel.slice(s, 0, 1, w))\n\
         neotmux.log.info(sel.slice(s, 4, 7, w))",
    );
    assert_eq!(logged(&sub), vec!["あい", "あ", "うえ"]);
}

/// A line that mixes widths is where a character index and a column drift apart: `a` is
/// column 0, `b` is 1, `あ` is 2 and 3, `c` is 4.
#[test]
fn a_mixed_line_counts_columns_and_not_characters() {
    let (_dir, sub) = built(
        "local s = 'ab\u{3042}cd'\n\
         neotmux.log.info(sel.slice(s, 0, 3, w))\n\
         neotmux.log.info(sel.slice(s, 4, 5, w))\n\
         neotmux.log.info(sel.slice(s, 0, 1, w))",
    );
    assert_eq!(logged(&sub), vec!["abあ", "cd", "ab"]);
}

/// A selection whose end lands inside a wide character takes that character: the column is
/// highlighted, so what is under it is selected. Both ends behave the same way.
#[test]
fn a_wide_character_the_selection_only_half_covers_is_still_in_it() {
    let (_dir, sub) = built(
        "local s = '\u{3042}\u{3044}\u{3046}'\n\
         neotmux.log.info(sel.slice(s, 1, 2, w))\n\
         neotmux.log.info(sel.slice(s, 0, 2, w))\n\
         neotmux.log.info(sel.slice(s, 3, 4, w))",
    );
    assert_eq!(logged(&sub), vec!["あい", "あい", "いう"]);
}

/// On a line of single-column characters a column and a character index are the same number,
/// so nothing about the old behaviour moves. This is the regression guard.
#[test]
fn an_ascii_line_slices_exactly_as_it_did() {
    let (_dir, sub) = built(
        "neotmux.log.info(sel.slice('hello', 0, 2, w))\n\
         neotmux.log.info(sel.slice('hello', 2, nil, w))\n\
         neotmux.log.info(sel.slice('hello', 0, nil, w))\n\
         neotmux.log.info('[' .. sel.slice('hello', 9, 12, w) .. ']')",
    );
    assert_eq!(logged(&sub), vec!["hel", "llo", "hello", "[]"]);
}

/// The whole point, end to end: what `span` says is highlighted and what `text` puts in the
/// register are the same region. `span` answers in columns; `text` must take the characters
/// under exactly those columns.
#[test]
fn what_the_highlight_covers_is_what_the_register_gets() {
    let (_dir, sub) = built(
        "local a = { line = 0, col = 0 }\n\
         local b = { line = 0, col = 3 }\n\
         local x, width = sel.span(a, b, 0, 80)\n\
         local got = sel.text(a, b, function() return '\u{3042}\u{3044}\u{3046}' end, w)\n\
         neotmux.log.info('columns ' .. x .. '..' .. (x + width - 1))\n\
         neotmux.log.info('register ' .. got .. ' (' .. w(got) .. ' columns)')",
    );
    assert_eq!(
        logged(&sub),
        vec!["columns 0..3", "register あい (4 columns)"]
    );
}

/// `text` spans lines, and each one is cut in the same coordinates.
#[test]
fn a_selection_across_lines_cuts_every_line_in_columns() {
    let (_dir, sub) = built(
        "local lines = { [0] = 'ab\u{3042}cd', [1] = '\u{3042}\u{3044}\u{3046}' }\n\
         local got = sel.text({ line = 0, col = 2 }, { line = 1, col = 1 },\n\
           function(n) return lines[n] end, w)\n\
         neotmux.log.info((got:gsub('\\n', '|')))",
    );
    assert_eq!(logged(&sub), vec!["あcd|あ"]);
}

/// The other three places issue #3 named, checked rather than assumed. `ctx:text` records a
/// run whole and the renderer advances by each character's width, so a label with a wide
/// character in it does not drift; `ctx:rect` carries no text and takes its width in columns
/// directly, so there is nothing in it to drift.
#[test]
fn a_text_run_is_recorded_whole_and_a_rect_carries_no_text() {
    use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size};

    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "neotmux.on_tick(function(ctx)\n\
         ctx:text(nil, 0, 0, 'a\u{3042}b')\n\
         ctx:rect(nil, 0, 1, 4, 1)\n\
         end)\n",
    )
    .unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());

    let size = Size { rows: 24, cols: 80 };
    let pane = PaneId::from(1);
    sub.begin_frame(
        support::one(),
        Epoch::new(size, vec![pane], Some(pane)),
        Geometry {
            area: Rect::new(0, 0, size.cols, size.rows),
            panes: vec![(pane, Rect::new(0, 0, 80, 24))],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());

    let cmds = sub.frame_output();
    let text = cmds
        .iter()
        .find_map(|cmd| match cmd {
            DrawCmd::Text { x, text, .. } => Some((*x, text.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the run was recorded: {cmds:?}"));
    assert_eq!(
        text,
        (0, "a\u{3042}b".to_string()),
        "the run crosses whole: nothing on this side steps a cursor per character"
    );
    assert!(
        cmds.iter().any(|cmd| matches!(cmd, DrawCmd::Rect { .. })),
        "a rect is a rect, with no text in it to misplace: {cmds:?}"
    );
}

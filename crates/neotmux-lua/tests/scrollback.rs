//! `neotmux.pane.scrollback` and `neotmux.pane.history`: the lines above a pane's screen.

use std::fs;

use neotmux_core::proto::{PaneId, Size};
use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;

/// Two rows, so the third line a pane prints is already history.
const SIZE: Size = Size { rows: 2, cols: 8 };

const REPORT: &str = r#"
local function report(id, opts)
  local lines, revision = neotmux.pane.scrollback(id, opts)
  if lines == nil then
    neotmux.log.info(id .. ' nil rev=' .. tostring(revision))
    return
  end
  neotmux.log.info(id .. ' [' .. table.concat(lines, '|') .. '] rev=' .. tostring(revision))
end

local function report_history(id)
  neotmux.log.info('history=' .. tostring(neotmux.pane.history(id)))
end

local function report_visible(id)
  local lines = neotmux.pane.content(id)
  neotmux.log.info('visible [' .. table.concat(lines, '|') .. ']')
end
"#;

fn built(body: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    let init = format!("{REPORT}\nneotmux.on_tick(function()\n{body}\nend)\n");
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn pane(id: u32) -> PaneId {
    PaneId::from(id)
}

fn ticked(sub: &LuaSubsystem) -> Vec<String> {
    sub.dispatch_tick();
    sub.drain_ops()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A pane with four lines printed into a two-row screen: two of history, two visible.
fn scrolled(sub: &LuaSubsystem) {
    sub.terminals().open(pane(1), SIZE, 100);
    sub.terminals()
        .feed(pane(1), b"one\r\ntwo\r\nthree\r\nfour");
}

#[test]
fn what_scrolled_off_the_top_is_what_the_scrollback_answers() {
    let (_dir, sub) = built("report(1)");
    scrolled(&sub);
    assert!(
        ticked(&sub)[0].starts_with("1 [one|two] rev="),
        "{:?}",
        ticked(&sub)
    );
}

/// The property a copy-mode plugin depends on: asking does not scroll the pane.
#[test]
fn reading_the_scrollback_leaves_the_visible_screen_where_it_was() {
    let (_dir, sub) = built("report_visible(1) report(1) report_visible(1)");
    scrolled(&sub);
    let lines = ticked(&sub);
    assert_eq!(lines[0], "visible [three|four]");
    assert_eq!(
        lines[2], "visible [three|four]",
        "the read moved the screen"
    );
}

#[test]
fn history_says_how_many_lines_are_above_the_screen() {
    let (_dir, sub) = built("report_history(1)");
    scrolled(&sub);
    assert_eq!(ticked(&sub)[0], "history=2");
}

#[test]
fn a_pane_that_has_not_scrolled_has_no_history_and_no_scrollback() {
    let (_dir, sub) = built("report_history(1) report(1)");
    sub.terminals().open(pane(1), SIZE, 100);
    sub.terminals().feed(pane(1), b"one");
    let lines = ticked(&sub);
    assert_eq!(lines[0], "history=0");
    assert!(lines[1].starts_with("1 [] rev="), "{:?}", lines[1]);
}

/// A number a plugin saved still means the same line after the child says more.
#[test]
fn new_output_does_not_renumber_a_line_already_in_the_history() {
    let (_dir, sub) = built("report(1, { from = 0, lines = 2 }) report_history(1)");
    scrolled(&sub);
    let before = ticked(&sub);
    sub.terminals().feed(pane(1), b"\r\nfive\r\nsix");
    let after = ticked(&sub);
    assert!(before[0].starts_with("1 [one|two] rev="), "{:?}", before[0]);
    assert!(after[0].starts_with("1 [one|two] rev="), "{:?}", after[0]);
    assert_eq!(before[1], "history=2");
    assert_eq!(after[1], "history=4");
}

#[test]
fn from_and_lines_pick_a_window_out_of_the_history() {
    let (_dir, sub) = built("report(1, { from = 1, lines = 2 })");
    sub.terminals().open(pane(1), SIZE, 100);
    sub.terminals().feed(pane(1), b"1\r\n2\r\n3\r\n4\r\n5\r\n6");
    assert!(ticked(&sub)[0].starts_with("1 [2|3] rev="));
}

#[test]
fn a_read_past_the_end_of_the_history_answers_an_empty_list() {
    let (_dir, sub) = built("report(1, { from = 99 })");
    scrolled(&sub);
    assert!(ticked(&sub)[0].starts_with("1 [] rev="));
}

/// The revision the scrollback is read at is the pane's, so the same gate serves for both
/// halves of reading a pane.
#[test]
fn the_revision_gates_a_scrollback_read_the_way_it_gates_content() {
    let (_dir, sub) = built("report(1) neotmux.log.info('rev=' .. neotmux.pane.revision(1))");
    scrolled(&sub);
    let first = ticked(&sub);
    let unchanged = ticked(&sub);
    assert_eq!(first[0], unchanged[0], "nothing was fed, nothing moved");
    assert_eq!(first[1], unchanged[1]);

    sub.terminals().feed(pane(1), b"\r\nfive");
    let after = ticked(&sub);
    assert_ne!(first[1], after[1], "a feed moved the revision");
}

#[test]
fn a_pane_nobody_has_answers_nil_rather_than_raising() {
    let (_dir, sub) = built("report(4242) report_history(4242)");
    let lines = ticked(&sub);
    assert_eq!(lines[0], "4242 nil rev=nil");
    assert_eq!(lines[1], "history=nil");
}

#[test]
fn a_negative_from_or_line_count_raises() {
    for opts in ["{ from = -1 }", "{ lines = -1 }"] {
        let (_dir, sub) = built(&format!("report(1, {opts})"));
        scrolled(&sub);
        let lines = ticked(&sub);
        assert!(
            lines.iter().any(|line| line.contains("of zero or more")),
            "{opts} did not raise: {lines:?}"
        );
    }
}

/// A rectangle selection that reaches into the history needs the same columns there as on
/// the screen, so `pad` reads the same way on both.
#[test]
fn the_pad_option_keeps_every_history_row_as_wide_as_the_pane() {
    let (_dir, sub) = built("report(1, { pad = true })");
    sub.terminals().open(pane(1), SIZE, 100);
    sub.terminals()
        .feed(pane(1), b"one\r\ntwo\r\nthree\r\nfour");

    let line = ticked(&sub).remove(0);
    assert!(
        line.starts_with("1 [one     |two     ] rev="),
        "a padded history row was not the pane's eight columns: {line:?}"
    );
}

/// A wrapped command line usually straddles the top of the screen, so the flags have to be
/// readable in the history on the same terms.
#[test]
fn the_wrapped_option_says_which_history_rows_run_on_into_the_next() {
    let (_dir, sub) = built(
        "local lines, _, wrapped = neotmux.pane.scrollback(1, { wrapped = true })\n\
         local flags = {}\n\
         for i = 1, #lines do flags[i] = tostring(wrapped[i]) end\n\
         neotmux.log.info(table.concat(flags, ','))",
    );
    sub.terminals()
        .open(pane(1), Size { rows: 2, cols: 4 }, 100);
    sub.terminals().feed(pane(1), b"abcdef\r\n\r\nxy");

    assert_eq!(ticked(&sub), vec!["true,false"]);
}

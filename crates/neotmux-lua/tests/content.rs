//! `neotmux.pane.content` and `neotmux.pane.revision`: what a pane is showing, read from Lua.

use std::fs;

use neotmux_core::proto::{PaneId, Size};
use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;

const SIZE: Size = Size { rows: 4, cols: 20 };

/// Reports a pane's lines joined by `|`, so one log line pins the whole answer. Every test
/// reads from a tick, which is where a watcher reads from.
const REPORT: &str = r#"
local function report(id, opts)
  local lines, revision = neotmux.pane.content(id, opts)
  if lines == nil then
    neotmux.log.info(id .. ' nil rev=' .. tostring(revision))
    return
  end
  neotmux.log.info(id .. ' [' .. table.concat(lines, '|') .. '] rev=' .. tostring(revision))
end
"#;

/// `body` runs on every tick, with `report` in scope.
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

/// One tick's worth of log lines.
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

#[test]
fn a_panes_content_is_what_was_written_to_it_as_lines() {
    let (_dir, sub) = built("report(1)");
    sub.terminals().open(pane(1), SIZE, 0);
    sub.terminals().feed(pane(1), b"hello\r\n\x1b[1;32mworld\x1b[0m");

    let line = ticked(&sub).remove(0);
    assert!(
        line.starts_with("1 [hello|world||] rev="),
        "escapes or blank rows leaked into {line:?}"
    );
}

#[test]
fn the_lines_option_reads_the_bottom_of_the_screen_only() {
    let (_dir, sub) = built("report(1, { lines = 2 })");
    sub.terminals().open(pane(1), SIZE, 0);
    sub.terminals().feed(pane(1), b"one\r\ntwo\r\nthree\r\nfour");

    assert!(ticked(&sub).remove(0).starts_with("1 [three|four] rev="));
}

#[test]
fn a_pane_nobody_has_answers_nil_rather_than_raising() {
    let (_dir, sub) = built("report(4242)");
    assert_eq!(ticked(&sub), vec!["4242 nil rev=nil"]);
}

/// A watcher runs in a loop, and a shell exiting between one pass and the next is not its
/// error.
#[test]
fn a_pane_that_closes_between_two_reads_answers_nil_for_the_second() {
    let (_dir, sub) = built("report(1)");
    sub.terminals().open(pane(1), SIZE, 0);
    sub.terminals().feed(pane(1), b"alive");
    assert!(ticked(&sub).remove(0).starts_with("1 [alive|"));

    sub.terminals().close(pane(1));
    assert_eq!(ticked(&sub), vec!["1 nil rev=nil"]);
}

#[test]
fn a_negative_lines_count_raises() {
    let (_dir, sub) = built("report(1, { lines = -1 })");
    sub.terminals().open(pane(1), SIZE, 0);

    let logged = ticked(&sub);
    assert!(
        logged
            .iter()
            .any(|line| line.contains("lines count of zero or more")),
        "unhelpful error: {logged:?}"
    );
}

/// The cost control, from Lua: a watcher asks this every frame and reads the lines only when
/// it moved.
#[test]
fn the_revision_moves_only_when_the_pane_has_been_fed() {
    let (_dir, sub) = built("neotmux.log.info(tostring(neotmux.pane.revision(1)))");
    sub.terminals().open(pane(1), SIZE, 0);

    let first = ticked(&sub).remove(0);
    assert_ne!(first, "nil");
    assert_eq!(ticked(&sub).remove(0), first, "nothing was fed");

    sub.terminals().feed(pane(1), b"x");
    assert_ne!(ticked(&sub).remove(0), first);

    sub.terminals().close(pane(1));
    assert_eq!(ticked(&sub).remove(0), "nil");
}

/// The revision `content` hands back is the one those lines were read at, so a watcher can
/// record exactly what it has seen.
#[test]
fn the_revision_content_reports_is_the_one_the_lines_were_read_at() {
    let (_dir, sub) = built("report(1) neotmux.log.info(tostring(neotmux.pane.revision(1)))");
    sub.terminals().open(pane(1), SIZE, 0);
    sub.terminals().feed(pane(1), b"x");

    let logged = ticked(&sub);
    let reported = logged[0].rsplit("rev=").next().expect("a revision");
    assert_eq!(reported, logged[1]);
}

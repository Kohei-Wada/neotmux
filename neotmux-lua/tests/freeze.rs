//! `neotmux.pane.freeze`: holding a pane still while its child keeps talking.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{GridSnapshot, PaneId, Size};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 2, cols: 8 };

const REPORT: &str = r#"
local function report(id)
  local lines, revision = neotmux.pane.content(id)
  local len, dropped = neotmux.pane.history(id)
  neotmux.log.info('[' .. table.concat(lines, '|') .. '] rev=' .. tostring(revision)
    .. ' history=' .. tostring(len) .. '+' .. tostring(dropped)
    .. ' frozen=' .. tostring(neotmux.pane.frozen(id)))
end
"#;

fn built(body: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    let init = format!("{REPORT}\nneotmux.on_tick(function()\n{body}\nend)\n");
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, Built::new(sub, terminals))
}

fn pane(id: u32) -> PaneId {
    PaneId::from(id)
}

fn ticked(sub: &LuaSubsystem) -> Vec<String> {
    sub.dispatch_tick(support::one());
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// What the host would draw: the grid it copies out of the pane's terminal every frame.
fn drawn(sub: &Built, id: u32) -> Vec<String> {
    let mut grid = GridSnapshot::default();
    sub.terminals().snapshot_into(pane(id), &mut grid);
    (0..grid.size.rows)
        .map(|row| {
            (0..grid.size.cols)
                .filter_map(|col| grid.cell(row, col))
                .map(|cell| {
                    if cell.text.is_empty() {
                        " ".to_string()
                    } else {
                        cell.text.clone()
                    }
                })
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The mutation this whole feature is: the display stops moving while the PTY keeps being
/// read, parsed and fed.
#[test]
fn the_display_stops_moving_while_the_pane_goes_on_being_fed() {
    let (_dir, sub) = built("report(1)");
    sub.terminals().open(pane(1), SIZE, 100);
    sub.terminals().feed(pane(1), b"before");

    assert_eq!(drawn(&sub, 1), vec!["before", ""]);
    let hold = sub.terminals().hold(pane(1)).expect("a live pane");

    sub.terminals().feed(pane(1), b"\r\nafter");
    assert_eq!(
        drawn(&sub, 1),
        vec!["before", ""],
        "the frozen pane drew what its child said after it was frozen"
    );

    assert!(sub.terminals().release(hold));
    assert_eq!(
        drawn(&sub, 1),
        vec!["before", "after"],
        "unfreezing lost what arrived while the pane was held"
    );
}

/// A plugin computing a selection wants the text it is drawing over. Handing it the live
/// text silently defeats the freeze.
#[test]
fn a_frozen_pane_reads_as_the_text_it_was_frozen_on() {
    let (_dir, sub) = built("report(1)");
    sub.terminals().open(pane(1), SIZE, 100);
    sub.terminals().feed(pane(1), b"1\r\n2\r\n3\r\n4");

    let before = ticked(&sub).remove(0);
    assert_eq!(before, "[3|4] rev=1 history=2+0 frozen=false");

    let hold = sub.terminals().hold(pane(1)).expect("a live pane");
    sub.terminals().feed(pane(1), b"\r\n5\r\n6\r\n7");
    assert_eq!(
        ticked(&sub).remove(0),
        "[3|4] rev=1 history=2+0 frozen=true",
        "a frozen pane answered about the live screen"
    );

    assert!(sub.terminals().release(hold));
    assert_eq!(
        ticked(&sub).remove(0),
        "[6|7] rev=2 history=5+0 frozen=false"
    );
}

#[test]
fn a_freeze_from_lua_is_a_handle_that_releases_once() {
    let (_dir, sub) = built(
        "local h = neotmux.pane.freeze(1)\n\
         neotmux.log.info(tostring(h.pane)\n\
         .. ' ' .. tostring(neotmux.pane.freeze(4242))\n\
         .. ' ' .. tostring(neotmux.pane.frozen(1))\n\
         .. ' ' .. tostring(h:release())\n\
         .. ' ' .. tostring(h.release())\n\
         .. ' ' .. tostring(neotmux.pane.frozen(1))\n\
         .. ' ' .. tostring(neotmux.pane.frozen(4242))\n\
         .. ' ' .. tostring(neotmux.pane.unfreeze))",
    );
    sub.terminals().open(pane(1), SIZE, 100);
    assert_eq!(ticked(&sub), vec!["1 nil true true false false nil nil"]);
}

/// Output goes on being dispatched while a pane is frozen: freezing decides what is drawn
/// and what is read, not whether the child is heard.
#[test]
fn a_frozen_pane_still_reports_its_output() {
    let (_dir, sub) = built("");
    sub.terminals().open(pane(1), SIZE, 100);
    sub.terminals().hold(pane(1));
    assert!(
        sub.terminals().feed(pane(1), b"\x1b[6n") == b"\x1b[1;1R",
        "a frozen pane stopped answering its child"
    );
}

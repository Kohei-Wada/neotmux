//! `neotmux.window.panes`: which panes a window holds, the hidden ones included.

use std::fs;

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{Geometry, LuaSubsystem, Window};
use neotmux_proto::{Epoch, PaneId, Rect, Size, WindowId};

mod support;

const SIZE: Size = Size { rows: 24, cols: 80 };

/// `body` runs on every tick, with `report` in scope: it says a window's panes joined by
/// `,`, so one log line pins the whole answer.
fn built(body: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    let init = format!(
        r#"
local function report(prefix, ids)
  if ids == nil then
    neotmux.log.info(prefix .. ' nil')
    return
  end
  neotmux.log.info(prefix .. ' ' .. table.concat(ids, ','))
end
neotmux.on_tick(function()
{body}
end)
"#
    );
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn pane(id: u32) -> PaneId {
    PaneId::from(id)
}

fn window(id: u32, panes: &[u32]) -> Window {
    Window {
        id: WindowId::from(id),
        panes: panes.iter().copied().map(pane).collect(),
    }
}

/// Window 1 is shown with two side-by-side panes; window 2 is hidden with two of its own.
fn publish(sub: &LuaSubsystem) {
    let shown = vec![
        (pane(1), Rect::new(0, 0, 40, 24)),
        (pane(3), Rect::new(40, 0, 40, 24)),
    ];
    sub.begin_frame(
        Epoch::new(SIZE, vec![pane(1), pane(3)], Some(pane(1))),
        Geometry {
            size: SIZE,
            panes: shown,
            focus: Some(pane(1)),
            windows: vec![window(1, &[1, 3]), window(2, &[2, 4])],
            window: Some(WindowId::from(1)),
        },
    );
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

#[test]
fn the_shown_windows_panes_are_the_ones_pane_list_gives() {
    let (_dir, sub) = built(
        "report('panes', neotmux.window.panes(neotmux.window.current()))\n\
         report('list', neotmux.pane.list())",
    );
    publish(&sub);
    assert_eq!(ticked(&sub), vec!["panes 1,3", "list 1,3"]);
}

/// The gap this closes: a pane in a window nobody is looking at is in no `pane.list()`, and
/// this is how a plugin finds it — in layout order, the order it will be shown in.
#[test]
fn a_hidden_windows_panes_are_answered_in_layout_order() {
    let (_dir, sub) = built("report('hidden', neotmux.window.panes(2))");
    publish(&sub);
    assert_eq!(ticked(&sub), vec!["hidden 2,4"]);
}

/// Nothing, the answer `pane.current` and `pane.rect` give — not an empty table, which no
/// living window ever has.
#[test]
fn a_window_that_does_not_exist_answers_nothing() {
    let (_dir, sub) = built("report('gone', neotmux.window.panes(99))");
    publish(&sub);
    assert_eq!(ticked(&sub), vec!["gone nil"]);
}

/// Finding, not acting: `pane.list` is still the session in front of the user, so a plugin
/// that walks it does not silently start walking every window.
#[test]
fn pane_list_stays_the_shown_windows_panes() {
    let (_dir, sub) = built("report('list', neotmux.pane.list())");
    publish(&sub);
    assert_eq!(ticked(&sub), vec!["list 1,3"]);
}

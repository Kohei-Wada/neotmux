//! `C-b ?`: every prefix key, the command it runs and what that does, in a list Enter runs
//! from — `plugin.prefix` on `neotmux.list`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size, WindowId, geom::Window};

mod support;

const SIZE: Size = Size {
    rows: 30,
    cols: 120,
};

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

/// A key of the test's own bound to a function, with the description a plugin binding one
/// gives it. `!` sorts before every key the prefix ships, so it is the first row.
const INIT: &str = "
local prefix = require('plugin.prefix')
prefix.commands['!'] = function() neotmux.log.info('ran bang') end
prefix.descriptions['!'] = 'say bang'
";

fn built() -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), INIT).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        support::panes(),
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
    (dir, sub)
}

fn press(sub: &LuaSubsystem, spec: &str) {
    assert_eq!(
        sub.dispatch_key(support::one(), spec),
        KeyOutcome::Consumed,
        "{spec}"
    );
}

/// One string per drawn row, in row order, runs joined.
fn rows(sub: &LuaSubsystem) -> Vec<String> {
    sub.dispatch_tick(support::one());
    let mut rows: BTreeMap<u16, Vec<(u16, String)>> = BTreeMap::new();
    for cmd in sub.frame_output() {
        if let DrawCmd::Text { x, y, text, .. } = cmd {
            rows.entry(y).or_default().push((x, text));
        }
    }
    rows.into_values()
        .map(|mut runs| {
            runs.sort_by_key(|(x, _)| *x);
            runs.into_iter().map(|(_, text)| text).collect::<String>()
        })
        .collect()
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::host::Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn the_keys_list_names_every_key_what_it_runs_and_what_that_does() {
    let (_dir, sub) = built();
    press(&sub, "C-b");
    press(&sub, "?");
    let shown = rows(&sub);
    let row = |needle: &str| {
        shown
            .iter()
            .find(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("no row holds {needle:?}:\n{}", shown.join("\n")))
            .clone()
    };
    let split = row("C-b v");
    assert!(split.contains("split-vertical"), "{split}");
    assert!(
        split.contains("split the focused pane, side by side"),
        "{split}"
    );
    let own = row("C-b !");
    assert!(own.contains("say bang"), "{own}");
    let itself = row("C-b ?");
    assert!(itself.contains("keys"), "{itself}");
    // Keys in order: `!` before `-` before `v`.
    let at = |needle: &str| shown.iter().position(|r| r.contains(needle)).expect(needle);
    assert!(
        at("C-b !") < at("C-b -") && at("C-b -") < at("C-b v"),
        "{shown:#?}"
    );
}

#[test]
fn enter_runs_the_key_under_the_cursor() {
    let (_dir, sub) = built();
    press(&sub, "C-b");
    press(&sub, "?");
    rows(&sub);
    logged(&sub);
    press(&sub, "Enter");
    assert_eq!(logged(&sub), vec!["ran bang".to_string()]);
    // And the list is gone: the next key is the pane's.
    assert_eq!(
        sub.dispatch_key(support::one(), "x"),
        KeyOutcome::Fallthrough
    );
}

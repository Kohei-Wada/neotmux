//! The shipped watcher, driven without a host: no `pane_output` event is ever dispatched
//! here, so the only way it can know a pane exists is by enumerating the windows.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{Geometry, LuaSubsystem, Window};
use neotmux_proto::{Epoch, PaneId, Rect, Size, WindowId};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 4, cols: 40 };
const PROMPT: &[u8] = b"Do you want to proceed? [y/N]";

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// The shipped runtime with the watcher on and the other three off, so the only ops a tick
/// can produce are the watcher's.
fn built() -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "local defaults = require('neotmux.defaults')\n\
         defaults.status = false\n\
         defaults.prefix = false\n\
         defaults.borders = false\n\
         defaults.watch = true\n",
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, Built::new(sub, terminals))
}

fn pane(id: u32) -> PaneId {
    PaneId::from(id)
}

/// Window 1 is shown with pane 1 focused; window 2 is hidden, holding pane 2.
fn publish(sub: &LuaSubsystem) {
    sub.begin_frame(
        Epoch::new(SIZE, vec![pane(1)], Some(pane(1))),
        Geometry {
            size: SIZE,
            panes: vec![(pane(1), Rect::new(0, 0, 40, 4))],
            focus: Some(pane(1)),
            windows: vec![
                Window {
                    id: WindowId::from(1),
                    panes: vec![pane(1)],
                },
                Window {
                    id: WindowId::from(2),
                    panes: vec![pane(2)],
                },
            ],
            window: Some(WindowId::from(1)),
        },
    );
}

fn notifications(sub: &LuaSubsystem) -> Vec<String> {
    sub.dispatch_tick();
    sub.drain_ops()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// The first-day failure this closes: the agent was started in a window nobody is looking at
/// and has printed nothing at all, so no `pane_output` ever named it. It is watched anyway,
/// and the prompt it eventually shows is reported.
#[test]
fn a_pane_that_has_never_produced_output_is_watched() {
    let (_dir, sub) = built();
    sub.terminals().open(pane(1), SIZE, 0);
    sub.terminals().open(pane(2), SIZE, 0);
    publish(&sub);

    // The pane has said nothing, so this tick is the watcher's only chance to hear of it.
    assert_eq!(notifications(&sub), Vec::<String>::new());

    sub.terminals().feed(pane(2), PROMPT);
    assert_eq!(
        notifications(&sub),
        vec!["pane 2 looks like it is waiting for you"]
    );
}

/// The gate is about how often a pane is read, not about which panes there are: a pane whose
/// terminal has not been fed since the last look is not read again, and the notification is
/// sent once rather than on every frame the prompt is still up.
#[test]
fn enumeration_does_not_report_the_same_prompt_twice() {
    let (_dir, sub) = built();
    sub.terminals().open(pane(1), SIZE, 0);
    sub.terminals().open(pane(2), SIZE, 0);
    publish(&sub);
    notifications(&sub);

    sub.terminals().feed(pane(2), PROMPT);
    assert_eq!(notifications(&sub).len(), 1);
    assert_eq!(notifications(&sub), Vec::<String>::new());
}

/// The pane in front of the user is the one you can already see is waiting.
#[test]
fn the_focused_pane_is_still_left_alone() {
    let (_dir, sub) = built();
    sub.terminals().open(pane(1), SIZE, 0);
    sub.terminals().open(pane(2), SIZE, 0);
    publish(&sub);
    notifications(&sub);

    sub.terminals().feed(pane(1), PROMPT);
    assert_eq!(notifications(&sub), Vec::<String>::new());
}

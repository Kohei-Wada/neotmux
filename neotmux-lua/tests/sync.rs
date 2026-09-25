//! `plugin.sync`: every key relayed to every pane of the window, and the one piece of it that
//! could not be Lua — `neotmux.pane.key`, which spells a keystroke the way the pane asked for.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::geometry::Window;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size, WindowId};

mod support;

const SIZE: Size = Size { rows: 24, cols: 80 };

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

/// Two panes in one window, with the shipped layer behind the test's own.
fn built(init: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    let panes = vec![PaneId::from(1), PaneId::from(2)];
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, panes.clone(), Some(PaneId::from(1))),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![
                (PaneId::from(1), Rect::new(0, 0, 40, 24)),
                (PaneId::from(2), Rect::new(40, 0, 40, 24)),
            ],
            focus: Some(PaneId::from(1)),
            windows: vec![Window {
                id: WindowId::from(1),
                panes,
                name: None,
            }],
            window: Some(WindowId::from(1)),
        },
    );
    (dir, sub)
}

const ON: &str = "require('neotmux.defaults').sync = true
     require('plugin.sync')";

fn keys_sent(sub: &LuaSubsystem) -> Vec<(u32, String)> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::PaneKey { pane, spec } => Some((pane.get(), spec)),
            _ => None,
        })
        .collect()
}

/// Off, a key is nobody's business but the host's: nothing is relayed and nothing is consumed.
#[test]
fn a_key_is_left_alone_until_it_is_turned_on() {
    let (_dir, sub) = built(ON);
    assert_eq!(
        sub.dispatch_key(support::one(), "x"),
        KeyOutcome::Fallthrough
    );
    assert!(keys_sent(&sub).is_empty());
}

/// On, every pane of the window gets it — the focused one included, which is why the key is
/// consumed rather than also being delivered by the host.
#[test]
fn every_pane_of_the_window_gets_the_key() {
    let (_dir, sub) = built(ON);
    require_on(&sub);
    assert_eq!(sub.dispatch_key(support::one(), "x"), KeyOutcome::Consumed);
    assert_eq!(
        keys_sent(&sub),
        vec![(1, "x".to_string()), (2, "x".to_string())]
    );
}

/// And off again, by the same command the palette runs.
#[test]
fn turning_it_off_gives_the_keyboard_back() {
    let (_dir, sub) = built(ON);
    require_on(&sub);
    let _ = keys_sent(&sub);
    run_sync(&sub);
    assert_eq!(
        sub.dispatch_key(support::one(), "x"),
        KeyOutcome::Fallthrough
    );
    assert!(keys_sent(&sub).is_empty());
}

/// The mode is raised while it is on, so whatever draws modes says so.
#[test]
fn the_mode_says_so_while_it_is_on() {
    let (_dir, sub) = built(&format!(
        "{ON}
         neotmux.on_tick(function()
           neotmux.log.info('mode=' .. tostring(require('neotmux.mode').current()))
         end)"
    ));
    require_on(&sub);
    sub.dispatch_tick(support::one());
    let said: Vec<String> = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(said.iter().any(|t| t == "mode=sync"), "{said:?}");
}

/// A spec nothing could mean is a programming error, the way an unparseable keymap spec is.
#[test]
fn a_key_spec_nothing_could_mean_raises() {
    let (_dir, sub) = built(
        "neotmux.on_key(function(key)
           if key == 'z' then
             neotmux.pane.key(1, 'Nonsense')
             return true
           end
           return false
         end)",
    );
    sub.dispatch_key(support::one(), "z");
    let said: Vec<String> = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(
        said.iter().any(|t| t.contains("cannot parse key spec")),
        "{said:?}"
    );
}

/// A pane the session does not hold is answered rather than queued.
#[test]
fn a_key_to_a_pane_that_is_not_there_is_refused() {
    let (_dir, sub) = built(
        "neotmux.on_key(function(key)
           if key == 'z' then
             local ok, why = neotmux.pane.key(99, 'x')
             neotmux.log.info(tostring(ok) .. ' ' .. tostring(why))
             return true
           end
           return false
         end)",
    );
    sub.dispatch_key(support::one(), "z");
    let ops = sub.drain_ops(Some(support::one()));
    let said: Vec<String> = ops
        .iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(said, vec!["false no pane %99".to_string()]);
    assert!(!ops.iter().any(|(_, op)| matches!(op, Op::PaneKey { .. })));
}

/// The command the palette runs, called by name the way the palette calls it.
fn run_sync(sub: &LuaSubsystem) {
    sub.run_command(support::one(), "sync-panes", &[]);
}

fn require_on(sub: &LuaSubsystem) {
    run_sync(sub);
}

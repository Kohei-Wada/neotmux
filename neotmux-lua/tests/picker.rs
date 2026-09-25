//! `plugin.sessions`: the session picker, pinned against the implementation as it stands
//! before it is rebuilt onto `neotmux.list`. Every assertion here is the contract the rebuild
//! has to keep — this file is not touched by that work.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{
    DrawCmd, Epoch, Geometry, PaneId, PaneInfo, Rect, SessionId, Size, WindowId, geom::Window,
};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 10, cols: 40 };

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

/// A layer of the test's own in front of the shipped one, so `plugin.sessions` is the real
/// file. The layer's whole job is opening the picker on `o`, since there is no `plugin.prefix`
/// here to bind `C-b s` to it.
const OPEN_ON_O: &str = "
local sessions = require('plugin.sessions')
neotmux.on_key(function(key)
  if key == 'o' then
    sessions.open()
    return true
  end
  return false
end)
";

/// `plugin.prefix` itself, wired to `plugin.sessions` the way a real `init.lua` would: needed
/// only by the test that pins what a real `C-b s` does, since every other test here opens the
/// picker through its own stand-in key.
const WITH_PREFIX: &str = "
require('plugin.prefix')
require('plugin.sessions')
";

fn built(init: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, Built::new(sub, terminals))
}

/// A session the host has named and given one pane running `command`, the way `sessions.rs`
/// makes one — with a `PaneInfo` published too, since the picker reads it.
fn make(sub: &Built, id: SessionId, name: &str, command: &str) {
    sub.name_session(id, name);
    let pane = PaneId::from(1);
    sub.publish_pane_info(
        id,
        BTreeMap::from([(
            pane,
            PaneInfo {
                command: Some(command.to_string()),
                cwd: None,
            },
        )]),
    );
    sub.begin_frame(
        id,
        Epoch::new(SIZE, vec![pane], Some(pane)),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane, Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane),
            windows: vec![Window {
                id: WindowId::from(1),
                panes: vec![pane],
                name: None,
            }],
            window: Some(WindowId::from(1)),
        },
    );
}

/// Two sessions to pick from: `work`, the one the test dispatches as, and `docs`, the other
/// one. Sorted, `docs` comes first — `neotmux.session.list()` is alphabetical regardless of
/// the order the sessions were made in.
fn two_sessions(sub: &Built) {
    make(sub, support::one(), "work", "bash");
    make(sub, SessionId::from(2), "docs", "vim");
}

fn open(sub: &Built) {
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        sub.dispatch_key(support::one(), spec);
    }
}

fn frame(sub: &LuaSubsystem) -> Vec<DrawCmd> {
    sub.dispatch_tick(support::one());
    sub.frame_output()
}

fn texts(cmds: &[DrawCmd]) -> Vec<String> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The session ops the picker raised, in the order it raised them — everything else the key
/// handler might log is not this file's concern.
fn session_ops(sub: &Built) -> Vec<Op> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::SessionCreate { .. }
            | Op::SessionSwitch { .. }
            | Op::SessionKill { .. }
            | Op::SessionRename { .. } => Some(op),
            _ => None,
        })
        .collect()
}

/// The picker draws every session, what it is running, how many panes it has, its own title,
/// and the hint line — with nothing typed into it.
#[test]
fn the_picker_draws_every_session_its_counts_and_the_hint_line() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);

    let said = texts(&frame(&sub));
    assert!(said.iter().any(|t| t.contains("sessions")), "{said:?}");
    assert!(said.iter().any(|t| t == "docs"), "{said:?}");
    assert!(said.iter().any(|t| t == "work"), "{said:?}");
    assert!(said.iter().any(|t| t == "vim  1p"), "{said:?}");
    assert!(said.iter().any(|t| t == "bash  1p"), "{said:?}");
    assert!(
        said.iter()
            .any(|t| t == "j k  enter  c new  r name  d kill"),
        "{said:?}"
    );
}

/// `Enter` switches to the session under the cursor and closes the picker. The cursor starts
/// on `docs`, alphabetically first and not the session this test is dispatched as.
#[test]
fn enter_switches_to_the_session_under_the_cursor_and_closes_the_picker() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);

    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
    assert_eq!(
        session_ops(&sub),
        vec![Op::SessionSwitch {
            name: "docs".to_string()
        }]
    );
    assert!(frame(&sub).is_empty(), "the picker did not close on Enter");
}

/// Choosing the session you are already in closes the picker same as any other, but raises no
/// switch: there is nowhere to switch to.
#[test]
fn choosing_the_current_session_closes_without_switching() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);
    keys(&sub, &["j"]); // docs (1) -> work (2), the session this test is dispatched as

    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
    assert!(
        session_ops(&sub).is_empty(),
        "a switch to the current session was raised"
    );
    assert!(frame(&sub).is_empty(), "the picker did not close on Enter");
}

/// `c`, a name, `Enter`: a session is created and switched to, in that order, and the picker
/// closes.
#[test]
fn c_creates_a_session_and_switches_to_it() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);
    keys(&sub, &["c", "n", "e", "w", "Enter"]);

    assert_eq!(
        session_ops(&sub),
        vec![
            Op::SessionCreate {
                name: "new".to_string()
            },
            Op::SessionSwitch {
                name: "new".to_string()
            },
        ]
    );
    assert!(
        frame(&sub).is_empty(),
        "the picker did not close after creating a session"
    );
}

/// `r`, a name typed over the one it is pre-filled with, `Enter`: the session under the cursor
/// is renamed, and the picker stays open.
#[test]
fn r_renames_the_session_under_the_cursor_and_stays_open() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);
    keys(
        &sub,
        &[
            "r",
            "Backspace",
            "Backspace",
            "Backspace",
            "Backspace",
            "w",
            "i",
            "k",
            "i",
            "Enter",
        ],
    );

    assert_eq!(
        session_ops(&sub),
        vec![Op::SessionRename {
            from: "docs".to_string(),
            to: "wiki".to_string(),
        }]
    );
    assert!(!frame(&sub).is_empty(), "the picker closed after a rename");
}

/// `d` ends the session under the cursor and stays open.
#[test]
fn d_kills_the_session_under_the_cursor() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);
    keys(&sub, &["d"]); // docs, under the cursor at the start

    assert_eq!(
        session_ops(&sub),
        vec![Op::SessionKill {
            name: "docs".to_string()
        }]
    );
    assert!(!frame(&sub).is_empty(), "the picker closed after a kill");
}

/// `d` is refused on the session you are in — ending that one would leave the picker with
/// nothing to close over.
#[test]
fn d_is_refused_on_the_current_session() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);
    keys(&sub, &["j", "d"]); // docs (1) -> work (2), the current session

    assert!(
        session_ops(&sub).is_empty(),
        "the current session was killed"
    );
    assert!(!frame(&sub).is_empty(), "the picker closed after a kill");
}

/// A second `C-b s` while the picker is up changes nothing: the picker is a float, and its own
/// key handler is offered before `plugin.prefix`'s and swallows every key while it is up, `C-b`
/// included — so `C-b` never re-arms the prefix and `s` never reaches it either. This is not a
/// regression from the rebuild: the old picker was a float with the same all-consuming key
/// handler and never closed on a second `C-b s` either. `Esc` is what leaves it, the same as
/// every other modal float here.
#[test]
fn a_second_c_b_s_does_nothing_while_the_picker_is_up_and_esc_still_closes_it() {
    let (_dir, sub) = built(WITH_PREFIX);
    two_sessions(&sub);
    keys(&sub, &["C-b", "s"]);
    assert!(!frame(&sub).is_empty(), "the picker did not open on C-b s");

    keys(&sub, &["C-b", "s"]);
    assert!(
        session_ops(&sub).is_empty(),
        "a second C-b s raised a session op"
    );
    assert!(!frame(&sub).is_empty(), "a second C-b s closed the picker");

    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    assert!(frame(&sub).is_empty(), "Esc did not close the picker");
}

/// `Esc` and `q` both close the picker with no session op raised.
#[test]
fn esc_and_q_close_the_picker_without_a_switch() {
    let (_dir, sub) = built(OPEN_ON_O);
    two_sessions(&sub);
    open(&sub);
    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    assert!(session_ops(&sub).is_empty());
    assert!(frame(&sub).is_empty(), "the picker did not close on Esc");

    open(&sub);
    assert_eq!(sub.dispatch_key(support::one(), "q"), KeyOutcome::Consumed);
    assert!(session_ops(&sub).is_empty());
    assert!(frame(&sub).is_empty(), "the picker did not close on q");
}

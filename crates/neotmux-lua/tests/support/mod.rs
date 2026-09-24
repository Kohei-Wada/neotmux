//! What a test has to hold that the host holds in the real thing: the panes the Lua layer
//! reads from. They are not the subsystem's own any more — the session owns them, and from
//! M3 the session is in the other process — so a test that writes into a pane has to keep a
//! handle to it.
#![allow(dead_code)]

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};
use neotmux_term::terminals::PaneTerminals;

/// Panes for a subsystem whose test never writes into one.
pub fn panes() -> Rc<PaneTerminals> {
    Rc::new(PaneTerminals::new())
}

/// A subsystem and the panes behind it. Derefs to the subsystem, so a test reads as though
/// it were holding one.
pub struct Built {
    lua: LuaSubsystem,
    terminals: Rc<PaneTerminals>,
}

impl Built {
    pub fn new(lua: LuaSubsystem, terminals: Rc<PaneTerminals>) -> Self {
        Self { lua, terminals }
    }

    pub fn terminals(&self) -> &PaneTerminals {
        &self.terminals
    }
}

impl Deref for Built {
    type Target = LuaSubsystem;

    fn deref(&self) -> &LuaSubsystem {
        &self.lua
    }
}

/// The session every test in this crate drives. They are about the Lua layer rather than
/// about sessions, and the layer's callers name one for every dispatch.
pub fn one() -> neotmux_proto::SessionId {
    neotmux_proto::SessionId::from(1)
}

/// The size shared by `detect.rs` and `state.rs`'s harness. Other test files pick their own —
/// this one is not a crate-wide default, only the one those two agree on.
pub const SIZE: Size = Size { rows: 16, cols: 40 };

pub fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// The one pane `detect.rs` and `state.rs` both drive.
pub fn pane() -> PaneId {
    PaneId::from(1)
}

/// The shipped runtime with every plugin off. `neotmux.detect` and `neotmux.state` are
/// convention modules — nothing registers them — so the only thing running here is what the
/// test's `init.lua` asks for. Shared by `detect.rs` and `state.rs`, which want the exact same
/// plugins-off list and must not drift apart from each other.
pub fn built(extra: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'guard', 'sync', 'watch', 'sidebar', 'footer',\n\
               'hints' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        one(),
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

pub fn feed(sub: &Built, text: &[u8]) {
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), text);
}

pub fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        assert_eq!(
            sub.dispatch_key(one(), spec),
            KeyOutcome::Consumed,
            "the test's own on_key did not take '{spec}'"
        );
    }
}

pub fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

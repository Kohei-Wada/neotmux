//! What a test has to hold that the host holds in the real thing: the panes the Lua layer
//! reads from. They are not the subsystem's own any more — the session owns them, and from
//! M3 the session is in the other process — so a test that writes into a pane has to keep a
//! handle to it.
#![allow(dead_code)]

use std::ops::Deref;
use std::rc::Rc;

use neotmux_lua::LuaSubsystem;
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

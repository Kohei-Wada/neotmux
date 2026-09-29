//! Turning an [`Event`] into what a Lua subscriber is handed.
//!
//! The event itself is a `proto` type: it is raised where the panes are and dispatched where
//! the plugins are, which from M3 are two processes. What it looks like in Lua is this
//! layer's business and stays here.

use mlua::{Lua, LuaString, Table};
use neotmux_proto::Event;

/// `pane_output`'s bytes are the only payload expensive enough to copy per subscriber, so
/// build the Lua string for it once, up front.
pub(crate) fn shared_bytes(event: &Event, lua: &Lua) -> mlua::Result<Option<LuaString>> {
    match event {
        Event::PaneOutput { bytes, .. } => Ok(Some(lua.create_string(bytes)?)),
        _ => Ok(None),
    }
}

/// A fresh table per subscriber: sharing one table would let a subscriber's edit
/// (e.g. `e.bytes = processed`) leak into what the next subscriber sees.
pub(crate) fn table(event: &Event, lua: &Lua, bytes: Option<&LuaString>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    match event {
        Event::PaneOutput { pane, .. } => {
            table.set("pane", pane.get())?;
            let bytes = bytes.expect("pane_output always carries a shared bytes string");
            table.set("bytes", bytes.clone())?;
        }
        Event::Resized { size } => {
            table.set("rows", size.rows)?;
            table.set("cols", size.cols)?;
        }
        Event::PaneOpened { pane }
        | Event::PaneClosed { pane }
        | Event::FocusChanged { pane }
        | Event::PaneChanged { pane }
        | Event::PaneInfoChanged { pane } => {
            table.set("pane", pane.get())?;
        }
        Event::WindowOpened { window }
        | Event::WindowClosed { window }
        | Event::WindowChanged { window }
        | Event::WindowRenamed { window } => {
            table.set("window", window.get())?;
        }
        Event::SessionCreated { session } | Event::SessionClosed { session } => {
            table.set("session", session.as_str())?;
        }
        Event::SessionRenamed { session, from } => {
            table.set("session", session.as_str())?;
            table.set("from", from.as_str())?;
        }
    }
    Ok(table)
}

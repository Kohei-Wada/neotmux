use mlua::{Lua, LuaString, Table};

use neotmux_core::proto::{PaneId, Size};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    PaneOutput { pane: PaneId, bytes: Vec<u8> },
    Resized { size: Size },
    PaneOpened { pane: PaneId },
    PaneClosed { pane: PaneId },
    FocusChanged { pane: PaneId },
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::PaneOutput { .. } => "pane_output",
            Event::Resized { .. } => "resized",
            Event::PaneOpened { .. } => "pane_opened",
            Event::PaneClosed { .. } => "pane_closed",
            Event::FocusChanged { .. } => "focus_changed",
        }
    }

    /// `pane_output`'s bytes are the only payload expensive enough to copy
    /// per subscriber, so build the Lua string for it once, up front.
    pub(crate) fn shared_bytes(&self, lua: &Lua) -> mlua::Result<Option<LuaString>> {
        match self {
            Event::PaneOutput { bytes, .. } => Ok(Some(lua.create_string(bytes)?)),
            _ => Ok(None),
        }
    }

    /// A fresh table per subscriber: sharing one table would let a
    /// subscriber's edit (e.g. `e.bytes = processed`) leak into what the
    /// next subscriber sees.
    pub(crate) fn table(&self, lua: &Lua, bytes: Option<&LuaString>) -> mlua::Result<Table> {
        let table = lua.create_table()?;
        match self {
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
            | Event::FocusChanged { pane } => {
                table.set("pane", pane.get())?;
            }
        }
        Ok(table)
    }
}

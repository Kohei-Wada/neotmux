use mlua::{Lua, LuaString, Table, Value};

use neotmux_core::proto::{PaneId, Size};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    PaneOpened { pane: PaneId },
    PaneClosed { pane: PaneId },
    PaneOutput { pane: PaneId, bytes: Vec<u8> },
    FocusChanged { pane: Option<PaneId> },
    Resized { size: Size },
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::PaneOpened { .. } => "pane_opened",
            Event::PaneClosed { .. } => "pane_closed",
            Event::PaneOutput { .. } => "pane_output",
            Event::FocusChanged { .. } => "focus_changed",
            Event::Resized { .. } => "resized",
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
            Event::PaneOpened { pane } | Event::PaneClosed { pane } => {
                table.set("pane", pane.get())?;
            }
            Event::PaneOutput { pane, .. } => {
                table.set("pane", pane.get())?;
                let bytes = bytes.expect("pane_output always carries a shared bytes string");
                table.set("bytes", bytes.clone())?;
            }
            Event::FocusChanged { pane } => {
                table.set(
                    "pane",
                    match pane {
                        Some(pane) => Value::Integer(i64::from(pane.get())),
                        None => Value::Nil,
                    },
                )?;
            }
            Event::Resized { size } => {
                table.set("rows", size.rows)?;
                table.set("cols", size.cols)?;
            }
        }
        Ok(table)
    }
}

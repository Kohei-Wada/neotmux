use mlua::{Lua, Table, Value};

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

    pub fn payload(&self, lua: &Lua) -> mlua::Result<Table> {
        let table = lua.create_table()?;
        match self {
            Event::PaneOpened { pane } | Event::PaneClosed { pane } => {
                table.set("pane", pane.get())?;
            }
            Event::PaneOutput { pane, bytes } => {
                table.set("pane", pane.get())?;
                table.set("bytes", lua.create_string(bytes)?)?;
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

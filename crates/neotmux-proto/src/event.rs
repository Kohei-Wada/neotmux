use crate::{PaneId, Size, WindowId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    PaneOutput { pane: PaneId, bytes: Vec<u8> },
    Resized { size: Size },
    PaneOpened { pane: PaneId },
    PaneClosed { pane: PaneId },
    FocusChanged { pane: PaneId },
    WindowOpened { window: WindowId },
    WindowClosed { window: WindowId },
    WindowChanged { window: WindowId },
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::PaneOutput { .. } => "pane_output",
            Event::Resized { .. } => "resized",
            Event::PaneOpened { .. } => "pane_opened",
            Event::PaneClosed { .. } => "pane_closed",
            Event::FocusChanged { .. } => "focus_changed",
            Event::WindowOpened { .. } => "window_opened",
            Event::WindowClosed { .. } => "window_closed",
            Event::WindowChanged { .. } => "window_changed",
        }
    }
}

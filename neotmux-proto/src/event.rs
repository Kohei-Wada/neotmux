use crate::{PaneId, Size, WindowId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    PaneOutput {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    Resized {
        size: Size,
    },
    PaneOpened {
        pane: PaneId,
    },
    PaneClosed {
        pane: PaneId,
    },
    FocusChanged {
        pane: PaneId,
    },
    WindowOpened {
        window: WindowId,
    },
    WindowClosed {
        window: WindowId,
    },
    WindowChanged {
        window: WindowId,
    },
    /// The pane's terminal was fed since the last frame. Once per pane per frame, with no
    /// bytes: `PaneOutput` is the per-chunk one for a plugin that reads them.
    PaneChanged {
        pane: PaneId,
    },
    WindowRenamed {
        window: WindowId,
    },
    SessionCreated {
        session: String,
    },
    /// Dispatched with no session bound: the session is on its way out.
    SessionClosed {
        session: String,
    },
    SessionRenamed {
        session: String,
        from: String,
    },
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
            Event::PaneChanged { .. } => "pane_changed",
            Event::WindowRenamed { .. } => "window_renamed",
            Event::SessionCreated { .. } => "session_created",
            Event::SessionClosed { .. } => "session_closed",
            Event::SessionRenamed { .. } => "session_renamed",
        }
    }
}

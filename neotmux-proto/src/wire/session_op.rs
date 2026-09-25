use crate::{Dir, PaneId, SplitDir, WindowId};

/// An op a plugin raised that the session has to apply. `log` and `notify` are not here:
/// they are raised and answered on the same side of the socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOp {
    Quit,
    /// The session is kept; only the client leaves.
    Detach,
    PaneSplit {
        dir: SplitDir,
    },
    PaneFocus {
        pane: PaneId,
    },
    PaneResize {
        dir: Dir,
        cells: u16,
    },
    /// Text into a pane's shell, as though it had been typed there. The pane is named rather
    /// than taken from the focus: a plugin sends to the pane it is answering, which is not
    /// usually the one under the cursor.
    PaneSend {
        pane: PaneId,
        text: String,
    },
    WindowCreate {
        window: WindowId,
    },
    WindowFocus {
        window: WindowId,
    },
    WindowRename {
        window: WindowId,
        name: Option<String>,
    },
}

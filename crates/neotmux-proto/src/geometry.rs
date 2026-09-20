use crate::{PaneId, Rect, Size, WindowId};

/// A window as Lua can ask about it: its id, and the panes it holds in layout order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    /// Left to right, top to bottom. A hidden window's order is the one it was last
    /// assigned, which is the arrangement it will be shown in.
    pub panes: Vec<PaneId>,
}

/// The session as Lua sees it: published by `begin_frame` and republished whenever the host
/// changes it mid-frame, so a callback always describes the session as it is. It carries no
/// epoch — that belongs to the frame's draw buffers, which must not be invalidated by a
/// change made after they were committed.
///
/// `panes` and `focus` are the **current** window's. A pane in a hidden window is not
/// in the list: Lua asks about the session in front of the user, and a plugin that wants to
/// know what else exists asks `window.list()` and `window.panes(id)`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Geometry {
    pub size: Size,
    pub panes: Vec<(PaneId, Rect)>,
    pub focus: Option<PaneId>,
    /// Every window, in creation order, with the panes it holds.
    pub windows: Vec<Window>,
    /// The one being shown. `None` only before the host has published anything.
    pub window: Option<WindowId>,
}

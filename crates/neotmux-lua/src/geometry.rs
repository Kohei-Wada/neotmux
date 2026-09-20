use neotmux_core::proto::{PaneId, Rect, Size, WindowId};

/// The session as Lua sees it: published by `begin_frame` and republished whenever the host
/// changes it mid-frame, so a callback always describes the session as it is. It carries no
/// epoch — that belongs to the frame's draw buffers, which must not be invalidated by a
/// change made after they were committed.
///
/// `panes` and `focus` are the **current** window's. A pane in a hidden window is not
/// in the list: Lua asks about the session in front of the user, and a plugin that wants to
/// know what else exists asks `window.list()`.
#[derive(Default, Clone)]
pub struct Geometry {
    pub size: Size,
    pub panes: Vec<(PaneId, Rect)>,
    pub focus: Option<PaneId>,
    /// Every window, in creation order.
    pub windows: Vec<WindowId>,
    /// The one being shown. `None` only before the host has published anything.
    pub window: Option<WindowId>,
}

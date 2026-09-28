use crate::geom::Window;
use crate::{PaneId, Rect, WindowId};

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
    /// The pane area, in the surface's own coordinates: a band reserved at the top pushes
    /// its `y` down and one at the left pushes its `x` across. A `pane = nil` draw is in
    /// those same coordinates, so this — not a bare size — is what a plugin anchors to.
    pub area: Rect,
    pub panes: Vec<(PaneId, Rect)>,
    pub focus: Option<PaneId>,
    /// Every window, in creation order, with the panes it holds.
    pub windows: Vec<Window>,
    /// The one being shown. `None` only before the host has published anything.
    pub window: Option<WindowId>,
    /// The pane zoomed over the whole area, if one is. `panes` is that pane alone while it
    /// is; the window's others are still in `windows`.
    pub zoomed: Option<PaneId>,
}

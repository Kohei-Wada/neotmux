use neotmux_core::proto::{PaneId, Rect, Size};

/// The tree as Lua sees it: published by `begin_frame` and republished whenever the host
/// changes it mid-frame, so a callback always describes the session as it is. It carries no
/// epoch — that belongs to the frame's draw buffers, which must not be invalidated by a
/// change made after they were committed.
#[derive(Default)]
pub(crate) struct Geometry {
    pub(crate) size: Size,
    pub(crate) panes: Vec<(PaneId, Rect)>,
    pub(crate) focus: Option<PaneId>,
}

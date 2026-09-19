use neotmux_core::proto::{Epoch, PaneId, Rect, Size};

/// What the host told Lua about this frame, captured once by `begin_frame` and read by
/// everything dispatched inside it.
#[derive(Default)]
pub(crate) struct Geometry {
    pub(crate) epoch: Epoch,
    pub(crate) size: Size,
    pub(crate) panes: Vec<(PaneId, Rect)>,
    pub(crate) focus: Option<PaneId>,
}

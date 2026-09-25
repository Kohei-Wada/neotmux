use crate::{GridSnapshot, PaneId, Rect};

/// One pane's place in a frame, and the copy of its screen to put there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneFrame<'a> {
    pub pane: PaneId,
    pub rect: Rect,
    pub grid: &'a GridSnapshot,
}

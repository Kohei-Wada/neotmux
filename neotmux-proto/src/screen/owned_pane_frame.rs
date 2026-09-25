use crate::{GridSnapshot, PaneId, Rect};

/// A [`PaneFrame`](crate::screen::PaneFrame) that owns its grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPaneFrame {
    pub pane: PaneId,
    pub rect: Rect,
    pub grid: GridSnapshot,
}

use crate::PaneId;

/// One caller's claim on a frozen pane. The pane stays frozen while any hold on it is
/// unreleased, so two callers freezing the same pane cannot thaw it for each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hold {
    pub pane: PaneId,
    id: u64,
}

impl Hold {
    /// For the implementor of [`PaneRead`](crate::screen::PaneRead): `id` is one it has
    /// never handed out before.
    pub fn new(pane: PaneId, id: u64) -> Self {
        Self { pane, id }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

use crate::{PaneId, Size};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Epoch {
    size: Size,
    panes: Vec<PaneId>,
    focus: Option<PaneId>,
}

impl Epoch {
    pub fn new(size: Size, mut panes: Vec<PaneId>, focus: Option<PaneId>) -> Self {
        panes.sort();
        Self { size, panes, focus }
    }

    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn panes(&self) -> &[PaneId] {
        &self.panes
    }

    pub fn focus(&self) -> Option<PaneId> {
        self.focus
    }
}

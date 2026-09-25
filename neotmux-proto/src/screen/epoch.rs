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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PaneId, Size};

    #[test]
    fn epochs_match_when_every_field_matches() {
        let a = Epoch::new(
            Size { rows: 24, cols: 80 },
            vec![PaneId::from(1)],
            Some(PaneId::from(1)),
        );
        let b = Epoch::new(
            Size { rows: 24, cols: 80 },
            vec![PaneId::from(1)],
            Some(PaneId::from(1)),
        );
        assert!(a.matches(&b));
    }

    #[test]
    fn a_resize_breaks_the_epoch() {
        let a = Epoch::new(Size { rows: 24, cols: 80 }, vec![], None);
        let b = Epoch::new(Size { rows: 25, cols: 80 }, vec![], None);
        assert!(!a.matches(&b));
    }

    #[test]
    fn a_closed_pane_breaks_the_epoch() {
        let a = Epoch::new(
            Size { rows: 24, cols: 80 },
            vec![PaneId::from(1), PaneId::from(2)],
            None,
        );
        let b = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1)], None);
        assert!(!a.matches(&b));
    }

    #[test]
    fn a_focus_change_breaks_the_epoch() {
        let a = Epoch::new(
            Size { rows: 24, cols: 80 },
            vec![PaneId::from(1)],
            Some(PaneId::from(1)),
        );
        let b = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1)], None);
        assert!(!a.matches(&b));
    }

    #[test]
    fn pane_order_does_not_affect_matching() {
        let a = Epoch::new(
            Size { rows: 1, cols: 1 },
            vec![PaneId::from(2), PaneId::from(1)],
            None,
        );
        let b = Epoch::new(
            Size { rows: 1, cols: 1 },
            vec![PaneId::from(1), PaneId::from(2)],
            None,
        );
        assert!(a.matches(&b));
    }
}

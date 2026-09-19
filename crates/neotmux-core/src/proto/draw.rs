use crate::proto::{Color, PaneId, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrawCmd {
    Text {
        pane: Option<PaneId>,
        x: u16,
        y: u16,
        text: String,
        fg: Color,
        bg: Color,
    },
    Rect {
        pane: Option<PaneId>,
        rect: Rect,
        fg: Color,
        bg: Color,
    },
    Line {
        pane: Option<PaneId>,
        from: (u16, u16),
        to: (u16, u16),
        fg: Color,
    },
    Marker {
        pane: Option<PaneId>,
        x: u16,
        y: u16,
        text: String,
        fg: Color,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct StatusFragment {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
}

impl StatusFragment {
    pub fn new(text: impl Into<String>, fg: Color, bg: Color) -> Self {
        Self {
            text: text.into(),
            fg,
            bg,
        }
    }
}

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{PaneId, Size};

    #[test]
    fn a_rect_knows_what_it_contains() {
        let r = Rect {
            x: 2,
            y: 3,
            w: 4,
            h: 2,
        };
        assert!(r.contains(2, 3));
        assert!(r.contains(5, 4));
        assert!(!r.contains(6, 4));
        assert!(!r.contains(5, 5));
        assert!(!r.contains(1, 3));
    }

    #[test]
    fn a_zero_sized_rect_contains_nothing() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
        assert!(!r.contains(0, 0));
    }

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

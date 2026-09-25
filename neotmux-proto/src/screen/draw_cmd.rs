use crate::screen::Rect;
use crate::{Color, PaneId};

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
    /// Recolours the cells of a rectangle and writes no glyph: whatever is under it stays
    /// legible. `None` for a channel leaves that channel as it was found.
    Tint {
        pane: Option<PaneId>,
        rect: Rect,
        fg: Option<Color>,
        bg: Option<Color>,
    },
    Line {
        pane: Option<PaneId>,
        from: (u16, u16),
        to: (u16, u16),
        /// One cell's worth: the renderer writes it into every cell of the line.
        glyph: char,
        fg: Color,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Epoch;
    use crate::{PaneId, Size};

    #[test]
    fn a_rect_knows_what_it_contains() {
        let r = Rect::new(2, 3, 4, 2);
        assert!(r.contains(2, 3));
        assert!(r.contains(5, 4));
        assert!(!r.contains(6, 4));
        assert!(!r.contains(5, 5));
        assert!(!r.contains(1, 3));
    }

    #[test]
    fn a_zero_sized_rect_contains_nothing() {
        let r = Rect::new(0, 0, 0, 0);
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

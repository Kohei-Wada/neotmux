use std::collections::HashMap;

use neotmux_core::layout::{SplitRefused, Tree, assign};
use neotmux_core::proto::{Epoch, PaneId, Rect as PRect, Size, SplitDir};
use neotmux_term::state::MIN_GRID;

/// Where each pane currently sits, in surface-local coordinates (the same
/// space `LuaSubsystem::begin_frame` was given).
pub type PaneRects = HashMap<PaneId, PRect>;

/// Everything the host knows about how panes are arranged: the tree (§6), the surface it is
/// assigned onto, and which pane has focus. Nothing outside this file decides how many panes
/// there are, how much of the surface each gets, or whether one is too small to show.
pub struct PaneLayout {
    /// The surface the panes share: the terminal minus the status row.
    pub content: Size,
    pub rects: PaneRects,
    /// How many cells to leave between panes. The host reserves them and has no idea what
    /// anyone puts there — the same deal as the status row.
    gutter: u16,
    tree: Tree,
    order: Vec<(PaneId, PRect)>,
    focus: Option<PaneId>,
}

/// A terminal at the minimum grid cannot spare a row for the status line.
fn content_size(full: Size, has_status: bool) -> Size {
    if has_status && full.rows > MIN_GRID.rows {
        Size {
            rows: full.rows - 1,
            cols: full.cols,
        }
    } else {
        full
    }
}

/// The one place "can this pane be shown at all" is decided. A rect below the minimum grid
/// belongs to a pane the terminal has shrunk out of room for (§6): it is hidden for the
/// frame, not resized to something `vt100` cannot hold and not closed.
fn showable(rect: PRect) -> bool {
    rect.w >= MIN_GRID.cols && rect.h >= MIN_GRID.rows
}

impl PaneLayout {
    pub fn new(full: Size, has_status: bool, gutter: u16, first: PaneId) -> Self {
        let mut layout = Self {
            content: Size::default(),
            rects: PaneRects::new(),
            gutter: 0,
            tree: Tree::new(first),
            order: Vec::new(),
            focus: Some(first),
        };
        layout.resurface(full, has_status, gutter);
        layout
    }

    /// The surface changed, whether from a terminal resize, the status row appearing, or the
    /// gutter between panes changing.
    pub fn resurface(&mut self, full: Size, has_status: bool, gutter: u16) {
        self.content = content_size(full, has_status);
        self.gutter = gutter;
        self.reassign();
    }

    fn reassign(&mut self) {
        self.order = assign(
            &self.tree,
            PRect::new(0, 0, self.content.cols, self.content.rows),
            self.gutter,
        );
        self.rects = self.order.iter().copied().collect();
    }

    /// Splits the focused pane, and focuses the new one — a split is made to be typed into.
    pub fn split(&mut self, dir: SplitDir, new: PaneId) -> Result<(), SplitRefused> {
        let target = self.focus.ok_or(SplitRefused::UnknownPane)?;
        let surface = PRect::new(0, 0, self.content.cols, self.content.rows);
        self.tree
            .split(target, dir, new, surface, MIN_GRID, self.gutter)?;
        self.reassign();
        self.focus = Some(new);
        Ok(())
    }

    /// Removes `pane`; its sibling takes the space. Returns the focus afterwards, which is
    /// `None` only when that was the last pane.
    pub fn close(&mut self, pane: PaneId) -> Option<PaneId> {
        let heir = self.tree.close(pane);
        self.reassign();
        if self.focus == Some(pane) {
            self.focus = heir;
        }
        if self.order.is_empty() {
            self.focus = None;
        }
        self.focus
    }

    pub fn focus(&self) -> Option<PaneId> {
        self.focus
    }

    /// `false` when no such pane exists, which the caller reports rather than acting on.
    pub fn set_focus(&mut self, pane: PaneId) -> bool {
        if !self.rects.contains_key(&pane) {
            return false;
        }
        self.focus = Some(pane);
        true
    }

    /// Every pane and its rect, left to right and top to bottom, as `begin_frame` wants them.
    pub fn panes(&self) -> Vec<(PaneId, PRect)> {
        self.order.clone()
    }

    /// The panes big enough to show this frame: the ones that get drawn and the ones whose
    /// PTY is resized. The others keep the size they last had.
    pub fn showable(&self) -> impl Iterator<Item = (PaneId, PRect)> + '_ {
        self.order.iter().copied().filter(|(_, r)| showable(*r))
    }

    pub fn epoch(&self) -> Epoch {
        Epoch::new(
            self.content,
            self.rects.keys().copied().collect(),
            self.focus,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u32) -> PaneId {
        PaneId::from(id)
    }

    fn layout(rows: u16, cols: u16) -> PaneLayout {
        PaneLayout::new(Size { rows, cols }, false, 0, pane(1))
    }

    #[test]
    fn without_a_status_row_the_pane_gets_the_whole_surface() {
        let full = Size { rows: 24, cols: 80 };
        let layout = PaneLayout::new(full, false, 0, pane(1));
        assert_eq!(layout.content, full);
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
    }

    #[test]
    fn a_status_row_costs_the_panes_one_row() {
        let layout = PaneLayout::new(Size { rows: 24, cols: 80 }, true, 0, pane(1));
        assert_eq!(layout.content, Size { rows: 23, cols: 80 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 23))]);
    }

    #[test]
    fn a_terminal_at_the_minimum_grid_cannot_spare_a_status_row() {
        let layout = PaneLayout::new(MIN_GRID, true, 0, pane(1));
        assert_eq!(layout.content, MIN_GRID);
    }

    #[test]
    fn a_split_halves_the_surface_and_focuses_the_new_pane() {
        let mut layout = layout(24, 80);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(40, 0, 40, 24)),
            ]
        );
        assert_eq!(layout.focus(), Some(pane(2)));
    }

    #[test]
    fn a_split_the_surface_cannot_hold_is_refused_and_changes_nothing() {
        let mut layout = layout(24, 2);
        assert_eq!(
            layout.split(SplitDir::Vertical, pane(2)),
            Err(SplitRefused::TooSmall)
        );
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 2, 24))]);
        assert_eq!(layout.focus(), Some(pane(1)));
    }

    #[test]
    fn closing_the_focused_pane_gives_its_space_and_the_focus_to_its_sibling() {
        let mut layout = layout(24, 80);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(layout.close(pane(2)), Some(pane(1)));
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
    }

    #[test]
    fn closing_an_unfocused_pane_leaves_the_focus_where_it_was() {
        let mut layout = layout(24, 80);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(layout.close(pane(1)), Some(pane(2)));
        assert_eq!(layout.focus(), Some(pane(2)));
    }

    #[test]
    fn closing_the_last_pane_leaves_nothing_focused() {
        let mut layout = layout(24, 80);
        assert_eq!(layout.close(pane(1)), None);
        assert!(layout.panes().is_empty());
    }

    #[test]
    fn focusing_an_unknown_pane_is_refused() {
        let mut layout = layout(24, 80);
        assert!(!layout.set_focus(pane(9)));
        assert_eq!(layout.focus(), Some(pane(1)));
        assert!(layout.set_focus(pane(1)));
    }

    /// §6: the terminal shrinking below what the tree can hold is the one case a refusal
    /// cannot prevent. The pane stays in the tree and keeps its place; it just is not shown.
    #[test]
    fn a_pane_below_the_minimum_is_hidden_rather_than_shown_at_that_size() {
        let mut layout = layout(24, 80);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        layout.resurface(Size { rows: 24, cols: 3 }, false, 0);

        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 2, 24)),
                (pane(2), PRect::new(2, 0, 1, 24)),
            ],
            "both panes keep their place in the tree"
        );
        assert_eq!(
            layout.showable().collect::<Vec<_>>(),
            vec![(pane(1), PRect::new(0, 0, 2, 24))],
            "the one-column pane is hidden for this frame"
        );

        layout.resurface(Size { rows: 24, cols: 80 }, false, 0);
        assert_eq!(layout.showable().count(), 2, "growing back brings it back");
    }

    /// The gutter is the host reserving space on a plugin's behalf, exactly as the status
    /// row is: the panes lose the cells and never learn what goes in them.
    #[test]
    fn a_gutter_costs_the_panes_the_cells_between_them() {
        let mut layout = PaneLayout::new(Size { rows: 24, cols: 80 }, false, 1, pane(1));
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(41, 0, 39, 24)),
            ]
        );

        layout.resurface(Size { rows: 24, cols: 80 }, false, 0);
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(40, 0, 40, 24)),
            ],
            "with the gutter back to zero the panes tile again"
        );
    }

    #[test]
    fn a_split_the_gutter_leaves_no_room_for_is_refused() {
        let mut layout = PaneLayout::new(Size { rows: 24, cols: 4 }, false, 1, pane(1));
        assert_eq!(
            layout.split(SplitDir::Vertical, pane(2)),
            Err(SplitRefused::TooSmall),
            "four columns hold two panes of two, but not with a gutter between them"
        );
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 4, 24))]);
    }

    #[test]
    fn the_epoch_names_the_content_size_the_panes_and_the_focus() {
        let mut layout = PaneLayout::new(Size { rows: 24, cols: 80 }, true, 0, pane(1));
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert!(layout.epoch().matches(&Epoch::new(
            Size { rows: 23, cols: 80 },
            vec![pane(1), pane(2)],
            Some(pane(2)),
        )));
    }
}

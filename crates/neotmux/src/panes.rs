use std::collections::HashMap;

use neotmux_core::layout::{ResizeRefused, SplitRefused, Tree, assign};
use neotmux_core::proto::{Dir, Epoch, PaneId, Rect as PRect, Reserved, Size, SplitDir};
use neotmux_term::state::MIN_GRID;

/// Where each pane currently sits, in surface-local coordinates (the same
/// space `LuaSubsystem::begin_frame` was given).
pub type PaneRects = HashMap<PaneId, PRect>;

/// Everything the host knows about how panes are arranged: the tree (§6), the surface it is
/// assigned onto, and which pane has focus. Nothing outside this file decides how many panes
/// there are, how much of the surface each gets, or whether one is too small to show.
pub struct PaneLayout {
    /// The surface the panes share: the terminal minus whatever bands are reserved at its
    /// edges.
    pub content: Size,
    pub rects: PaneRects,
    /// The rows the reserved bands are actually taking this frame, which is either every
    /// granted row or none of them.
    applied: Reserved,
    /// How many cells to leave between panes. The host reserves them and has no idea what
    /// anyone puts there — the same deal as a band at the edge.
    gutter: u16,
    tree: Tree,
    order: Vec<(PaneId, PRect)>,
    focus: Option<PaneId>,
}

/// The one place a reserved band costs the panes anything, and the one place it is decided
/// whether the terminal can afford it at all.
///
/// A surface that cannot spare the reserved rows and still leave the panes a grid `vt100`
/// survives applies **none** of them, rather than part of one: a half-honoured band is a band
/// its owner would draw outside. That is the §6 hidden-pane case one level up — the bands are
/// not shown for the frame, nothing is refused and nothing is torn down, and growing the
/// terminal back brings them straight back.
fn applied(full: Size, reserved: Reserved) -> Reserved {
    if full.rows >= reserved.rows().saturating_add(MIN_GRID.rows) {
        reserved
    } else {
        Reserved::default()
    }
}

/// The one place "can this pane be shown at all" is decided. A rect below the minimum grid
/// belongs to a pane the terminal has shrunk out of room for (§6): it is hidden for the
/// frame, not resized to something `vt100` cannot hold and not closed.
fn showable(rect: PRect) -> bool {
    rect.w >= MIN_GRID.cols && rect.h >= MIN_GRID.rows
}

impl PaneLayout {
    pub fn new(full: Size, reserved: Reserved, gutter: u16, first: PaneId) -> Self {
        let mut layout = Self {
            content: Size::default(),
            rects: PaneRects::new(),
            applied: Reserved::default(),
            gutter: 0,
            tree: Tree::new(first),
            order: Vec::new(),
            focus: Some(first),
        };
        layout.resurface(full, reserved, gutter);
        layout
    }

    /// The surface changed, whether from a terminal resize, a band being reserved or
    /// released, or the gutter between panes changing.
    pub fn resurface(&mut self, full: Size, reserved: Reserved, gutter: u16) {
        self.applied = applied(full, reserved);
        self.content = Size {
            rows: full.rows - self.applied.rows(),
            cols: full.cols,
        };
        self.gutter = gutter;
        self.reassign();
    }

    /// Whether the bands the sources were granted are the ones being drawn this frame. False
    /// means the terminal could not afford them and every band is hidden.
    pub fn bands_applied(&self, reserved: Reserved) -> bool {
        self.applied == reserved
    }

    /// The pane area, in the surface's own coordinates: a top band pushes the panes down, so
    /// a pane rect is never relative to anything but the terminal itself.
    fn surface(&self) -> PRect {
        PRect::new(0, self.applied.top, self.content.cols, self.content.rows)
    }

    fn reassign(&mut self) {
        self.order = assign(&self.tree, self.surface(), self.gutter);
        self.rects = self.order.iter().copied().collect();
    }

    /// Splits the focused pane, and focuses the new one — a split is made to be typed into.
    pub fn split(&mut self, dir: SplitDir, new: PaneId) -> Result<(), SplitRefused> {
        let target = self.focus.ok_or(SplitRefused::UnknownPane)?;
        let surface = self.surface();
        self.tree
            .split(target, dir, new, surface, MIN_GRID, self.gutter)?;
        self.reassign();
        self.focus = Some(new);
        Ok(())
    }

    /// Moves the boundary between the focused pane and its neighbour in `dir` by `cells`.
    pub fn resize(&mut self, dir: Dir, cells: u16) -> Result<(), ResizeRefused> {
        let target = self.focus.ok_or(ResizeRefused::UnknownPane)?;
        let surface = self.surface();
        self.tree
            .resize(target, dir, cells, surface, MIN_GRID, self.gutter)?;
        self.reassign();
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
        PaneLayout::new(Size { rows, cols }, Reserved::default(), 0, pane(1))
    }

    fn bottom(rows: u16) -> Reserved {
        Reserved::new(0, rows)
    }

    #[test]
    fn with_nothing_reserved_the_pane_gets_the_whole_surface() {
        let full = Size { rows: 24, cols: 80 };
        let layout = PaneLayout::new(full, Reserved::default(), 0, pane(1));
        assert_eq!(layout.content, full);
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
    }

    #[test]
    fn a_reserved_bottom_band_costs_the_panes_exactly_its_rows() {
        let layout = PaneLayout::new(Size { rows: 24, cols: 80 }, bottom(2), 0, pane(1));
        assert_eq!(layout.content, Size { rows: 22, cols: 80 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 22))]);
    }

    /// A band at the top is the one that moves the panes rather than only shortening them,
    /// which is why pane rects are in the surface's coordinates and not the pane area's.
    #[test]
    fn a_reserved_top_band_moves_the_panes_down_by_its_rows() {
        let layout = PaneLayout::new(Size { rows: 24, cols: 80 }, Reserved::new(1, 0), 0, pane(1));
        assert_eq!(layout.content, Size { rows: 23, cols: 80 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 1, 80, 23))]);
    }

    #[test]
    fn both_edges_are_subtracted_from_the_same_surface() {
        let layout = PaneLayout::new(Size { rows: 24, cols: 80 }, Reserved::new(1, 2), 0, pane(1));
        assert_eq!(layout.content, Size { rows: 21, cols: 80 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 1, 80, 21))]);
    }

    /// The reservation is all or nothing: a surface that cannot leave the panes a grid
    /// `vt100` holds applies none of it, rather than honouring part of a band its owner
    /// would then draw outside.
    #[test]
    fn a_terminal_too_small_for_the_bands_applies_none_of_them() {
        let reserved = bottom(1);
        let mut layout = PaneLayout::new(MIN_GRID, reserved, 0, pane(1));
        assert_eq!(layout.content, MIN_GRID);
        assert!(!layout.bands_applied(reserved));

        layout.resurface(
            Size {
                rows: MIN_GRID.rows + 1,
                cols: MIN_GRID.cols,
            },
            reserved,
            0,
        );
        assert_eq!(
            layout.content, MIN_GRID,
            "growing back brings the band back"
        );
        assert!(layout.bands_applied(reserved));
    }

    #[test]
    fn releasing_a_reservation_gives_the_panes_their_rows_back() {
        let mut layout = PaneLayout::new(Size { rows: 24, cols: 80 }, bottom(1), 0, pane(1));
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 23))]);
        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0);
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
    }

    /// A band is outside every pane's rect, so nothing a pane's child writes can land in it.
    #[test]
    fn no_pane_rect_overlaps_a_reserved_band() {
        let mut layout =
            PaneLayout::new(Size { rows: 24, cols: 80 }, Reserved::new(1, 1), 0, pane(1));
        layout.split(SplitDir::Horizontal, pane(2)).expect("fits");
        for (id, rect) in layout.panes() {
            assert!(rect.y >= 1, "{id} starts inside the top band: {rect:?}");
            assert!(
                rect.y + rect.h <= 23,
                "{id} runs into the bottom band: {rect:?}"
            );
        }
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

    /// The op is about the focused pane, so the layout is where "which pane" is decided.
    #[test]
    fn a_resize_moves_the_focused_panes_boundary_and_leaves_the_focus_alone() {
        let mut layout = layout(24, 80);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        layout.resize(Dir::Left, 6).expect("there is room");
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 34, 24)),
                (pane(2), PRect::new(34, 0, 46, 24)),
            ]
        );
        assert_eq!(layout.focus(), Some(pane(2)));
    }

    #[test]
    fn a_resize_with_no_neighbour_that_way_is_refused_and_changes_nothing() {
        let mut layout = layout(24, 80);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(
            layout.resize(Dir::Right, 1),
            Err(ResizeRefused::NoNeighbour)
        );
        assert_eq!(
            layout.resize(Dir::Left, 80),
            Err(ResizeRefused::TooSmall),
            "a move that leaves the other side below the minimum grid"
        );
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(40, 0, 40, 24)),
            ]
        );
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
        layout.resurface(Size { rows: 24, cols: 3 }, Reserved::default(), 0);

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

        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0);
        assert_eq!(layout.showable().count(), 2, "growing back brings it back");
    }

    /// The gutter is the host reserving space on a plugin's behalf, exactly as a band at the
    /// edge is: the panes lose the cells and never learn what goes in them.
    #[test]
    fn a_gutter_costs_the_panes_the_cells_between_them() {
        let mut layout =
            PaneLayout::new(Size { rows: 24, cols: 80 }, Reserved::default(), 1, pane(1));
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(41, 0, 39, 24)),
            ]
        );

        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0);
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
        let mut layout =
            PaneLayout::new(Size { rows: 24, cols: 4 }, Reserved::default(), 1, pane(1));
        assert_eq!(
            layout.split(SplitDir::Vertical, pane(2)),
            Err(SplitRefused::TooSmall),
            "four columns hold two panes of two, but not with a gutter between them"
        );
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 4, 24))]);
    }

    #[test]
    fn the_epoch_names_the_content_size_the_panes_and_the_focus() {
        let mut layout = PaneLayout::new(Size { rows: 24, cols: 80 }, bottom(1), 0, pane(1));
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert!(layout.epoch().matches(&Epoch::new(
            Size { rows: 23, cols: 80 },
            vec![pane(1), pane(2)],
            Some(pane(2)),
        )));
    }
}

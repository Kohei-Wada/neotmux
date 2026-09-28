use crate::{ResizeRefused, SplitRefused, Tree, assign};
use neotmux_proto::{Dir, Edge, Epoch, PaneId, PaneRects, Rect as PRect, Reserved, Size, SplitDir};

/// Everything the host knows about how panes are arranged: the tree (§6), the surface it is
/// assigned onto, and which pane has focus. Nothing outside this file decides how many panes
/// there are, how much of the surface each gets, or whether one is too small to show.
pub struct PaneLayout {
    /// The surface the panes share: the terminal minus whatever bands are reserved at its
    /// edges.
    pub content: Size,
    pub rects: PaneRects,
    /// What the reserved bands are actually taking this frame, which on each axis is either
    /// every granted cell of it or none.
    applied: Reserved,
    /// How many cells to leave between panes. The host reserves them and has no idea what
    /// anyone puts there — the same deal as a band at the edge.
    gutter: u16,
    /// The columns taken off the left of every pane's content, which the pane's outer rect
    /// still covers. A pane too narrow to spare them keeps its outer rect as its content.
    pane_band: u16,
    /// The smallest a pane may be, as the caller gave it. Not a constant here: this file does
    /// not know a terminal exists, so it takes the minimum on trust from whoever does and
    /// passes it in.
    minimum: Size,
    tree: Tree,
    order: Vec<(PaneId, PRect)>,
    focus: Option<PaneId>,
    /// The pane given the whole area while the others are hidden, tmux's zoom. The tree and
    /// `rects` are left as they are, so every other pane keeps its place and its shell, and
    /// unzooming puts them back where they were.
    zoomed: Option<PaneId>,
    /// The rects of the panes being shown: `rects` while nothing is zoomed, the zoomed pane
    /// over the whole area while one is. What a frame is drawn against.
    pub shown: PaneRects,
}

/// The one place a reserved band costs the panes anything, and the one place it is decided
/// whether the terminal can afford it at all.
///
/// A surface that cannot spare the reserved cells and still leave the panes a grid at least
/// the minimum the caller gave applies **none** of them, rather than part of one: a
/// half-honoured band is a band its owner would draw outside. That is the §6 hidden-pane
/// case one level up — the bands are
/// not shown for the frame, nothing is refused and nothing is torn down, and growing the
/// terminal back brings them straight back.
///
/// All-or-nothing holds per axis, not across both: rows and columns are spent out of
/// different room, so a terminal too short for the status line can still be wide enough for
/// the sidebar, and dropping the sidebar too would give the panes nothing back for it.
fn applied(full: Size, reserved: Reserved, minimum: Size) -> Reserved {
    let mut applied = Reserved::default();
    if full.rows >= reserved.rows().saturating_add(minimum.rows) {
        applied.add(Edge::Top, reserved.top);
        applied.add(Edge::Bottom, reserved.bottom);
    }
    if full.cols >= reserved.cols().saturating_add(minimum.cols) {
        applied.add(Edge::Left, reserved.left);
        applied.add(Edge::Right, reserved.right);
    }
    applied
}

/// The one place "can this pane be shown at all" is decided. A rect below the minimum the
/// caller gave belongs to a pane the terminal has shrunk out of room for (§6): it is hidden
/// for the frame, not resized to something smaller than that minimum and not closed.
fn showable(rect: PRect, minimum: Size) -> bool {
    rect.w >= minimum.cols && rect.h >= minimum.rows
}

/// The one place a pane band costs a pane anything: its content is its outer rect less the
/// band, or the whole outer rect for a pane too narrow to spare it. Per pane, unlike
/// `applied()`, so that one small pane does not take every other pane's band away with it.
fn inset(outer: PRect, band: u16, minimum: Size) -> PRect {
    if band == 0 || outer.w < minimum.cols.saturating_add(band) {
        return outer;
    }
    PRect::new(outer.x + band, outer.y, outer.w - band, outer.h)
}

impl PaneLayout {
    pub fn new(
        full: Size,
        reserved: Reserved,
        gutter: u16,
        pane_band: u16,
        first: PaneId,
        minimum: Size,
    ) -> Self {
        let mut layout = Self {
            content: Size::default(),
            rects: PaneRects::new(),
            applied: Reserved::default(),
            gutter: 0,
            pane_band: 0,
            minimum,
            tree: Tree::new(first),
            order: Vec::new(),
            focus: Some(first),
            zoomed: None,
            shown: PaneRects::new(),
        };
        layout.resurface(full, reserved, gutter, pane_band);
        layout
    }

    /// The surface changed, whether from a terminal resize, a band being reserved or
    /// released, the gutter between panes changing, or the pane band changing.
    pub fn resurface(&mut self, full: Size, reserved: Reserved, gutter: u16, pane_band: u16) {
        self.applied = applied(full, reserved, self.minimum);
        self.content = Size {
            rows: full.rows - self.applied.rows(),
            cols: full.cols - self.applied.cols(),
        };
        self.gutter = gutter;
        self.pane_band = pane_band;
        self.reassign();
    }

    /// The bands being drawn this frame, which is what the sources were granted minus any
    /// axis the terminal could not afford.
    pub fn applied_bands(&self) -> Reserved {
        self.applied
    }

    /// The pane area, in the surface's own coordinates: a top band pushes the panes down and
    /// a left band pushes them across, so a pane rect is never relative to anything but the
    /// terminal itself.
    pub fn area(&self) -> PRect {
        PRect::new(
            self.applied.left,
            self.applied.top,
            self.content.cols,
            self.content.rows,
        )
    }

    fn reassign(&mut self) {
        self.order = assign(&self.tree, self.area(), self.gutter);
        self.rects = self
            .order
            .iter()
            .map(|(id, outer)| (*id, inset(*outer, self.pane_band, self.minimum)))
            .collect();
        self.reshow();
    }

    /// The smallest a pane may be cut to: the minimum its shell needs, plus the band it gives
    /// up, so a split cannot leave a pane that is only a band.
    fn cut_minimum(&self) -> Size {
        Size {
            rows: self.minimum.rows,
            cols: self.minimum.cols.saturating_add(self.pane_band),
        }
    }

    fn reshow(&mut self) {
        self.shown = self.panes().into_iter().collect();
    }

    /// Splits the focused pane, and focuses the new one — a split is made to be typed into.
    pub fn split(&mut self, dir: SplitDir, new: PaneId) -> Result<(), SplitRefused> {
        let target = self.focus.ok_or(SplitRefused::UnknownPane)?;
        let surface = self.area();
        self.tree
            .split(target, dir, new, surface, self.cut_minimum(), self.gutter)?;
        // The new pane has to be seen; a refused split changes nothing, the zoom included.
        self.zoomed = None;
        self.reassign();
        self.focus = Some(new);
        Ok(())
    }

    /// Moves the boundary between the focused pane and its neighbour in `dir` by `cells`.
    pub fn resize(&mut self, dir: Dir, cells: u16) -> Result<(), ResizeRefused> {
        let target = self.focus.ok_or(ResizeRefused::UnknownPane)?;
        let surface = self.area();
        self.tree
            .resize(target, dir, cells, surface, self.cut_minimum(), self.gutter)?;
        // The boundary moved is one nobody could see while a pane filled the area.
        self.zoomed = None;
        self.reassign();
        Ok(())
    }

    /// Removes `pane`; its sibling takes the space. Returns the focus afterwards, which is
    /// `None` only when that was the last pane.
    pub fn close(&mut self, pane: PaneId) -> Option<PaneId> {
        if self.zoomed == Some(pane) {
            self.zoomed = None;
        }
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
    /// Focusing a pane the zoom is hiding unzooms, as tmux does: a pane you cannot see is not
    /// one to type into.
    pub fn set_focus(&mut self, pane: PaneId) -> bool {
        if !self.rects.contains_key(&pane) {
            return false;
        }
        if self.zoomed.is_some_and(|zoomed| zoomed != pane) {
            self.zoomed = None;
            self.reshow();
        }
        self.focus = Some(pane);
        true
    }

    /// Zooms the focused pane, or unzooms. A window of one pane has nothing to hide, and stays
    /// as it is. Answers whether a pane is zoomed afterwards.
    pub fn toggle_zoom(&mut self) -> bool {
        self.zoomed = match self.zoomed {
            Some(_) => None,
            None if self.order.len() > 1 => self.focus,
            None => None,
        };
        self.reshow();
        self.zoomed.is_some()
    }

    pub fn zoomed(&self) -> Option<PaneId> {
        self.zoomed
    }

    /// Every pane being shown and its outer rect — what the tree cut, bands and all — left to
    /// right and top to bottom: the zoomed pane alone, over the whole area, while one is.
    pub fn outer_panes(&self) -> Vec<(PaneId, PRect)> {
        match self.zoomed {
            Some(zoomed) => vec![(zoomed, self.area())],
            None => self.order.clone(),
        }
    }

    /// The same panes with their content rects, as `begin_frame` wants them: what each pane's
    /// shell is sized to and where its grid is drawn.
    pub fn panes(&self) -> Vec<(PaneId, PRect)> {
        self.outer_panes()
            .into_iter()
            .map(|(id, outer)| (id, inset(outer, self.pane_band, self.minimum)))
            .collect()
    }

    /// Every pane in the window, in layout order, zoomed or not: what the window holds rather
    /// than what it shows.
    pub fn arranged(&self) -> Vec<PaneId> {
        self.order.iter().map(|(id, _)| *id).collect()
    }

    /// The panes big enough to show this frame: the ones that get drawn and the ones whose
    /// PTY is resized. The others keep the size they last had — which is what a pane the
    /// zoom hides does too.
    pub fn showable(&self) -> impl Iterator<Item = (PaneId, PRect)> + '_ {
        let minimum = self.minimum;
        self.panes()
            .into_iter()
            .filter(move |(_, r)| showable(*r, minimum))
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
        PaneLayout::new(
            Size { rows, cols },
            Reserved::default(),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        )
    }

    fn bottom(rows: u16) -> Reserved {
        Reserved::horizontal(0, rows)
    }

    #[test]
    fn with_nothing_reserved_the_pane_gets_the_whole_surface() {
        let full = Size { rows: 24, cols: 80 };
        let layout = PaneLayout::new(
            full,
            Reserved::default(),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, full);
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
    }

    #[test]
    fn a_reserved_bottom_band_costs_the_panes_exactly_its_rows() {
        let layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            bottom(2),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, Size { rows: 22, cols: 80 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 22))]);
    }

    /// A band at the top is the one that moves the panes rather than only shortening them,
    /// which is why pane rects are in the surface's coordinates and not the pane area's.
    #[test]
    fn a_reserved_top_band_moves_the_panes_down_by_its_rows() {
        let layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::horizontal(1, 0),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, Size { rows: 23, cols: 80 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 1, 80, 23))]);
    }

    #[test]
    fn both_edges_are_subtracted_from_the_same_surface() {
        let layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::horizontal(1, 2),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, Size { rows: 21, cols: 80 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 1, 80, 21))]);
    }

    #[test]
    fn a_reserved_left_band_costs_the_panes_exactly_its_columns() {
        let layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::vertical(20, 0),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, Size { rows: 24, cols: 60 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(20, 0, 60, 24))]);
    }

    #[test]
    fn a_reserved_right_band_shortens_the_panes_without_moving_them() {
        let layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::vertical(0, 20),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, Size { rows: 24, cols: 60 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 60, 24))]);
    }

    #[test]
    fn both_vertical_edges_are_subtracted_from_the_same_surface() {
        let layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::vertical(20, 10),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, Size { rows: 24, cols: 50 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(20, 0, 50, 24))]);
    }

    /// The two axes are spent out of different room, so the pane area is the surface minus
    /// both of them.
    #[test]
    fn a_band_on_each_axis_costs_the_panes_both() {
        let mut reserved = Reserved::horizontal(1, 1);
        reserved.add(Edge::Left, 20);
        let layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            reserved,
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.content, Size { rows: 22, cols: 60 });
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(20, 1, 60, 22))]);
    }

    #[test]
    fn releasing_a_vertical_reservation_gives_the_panes_their_columns_back() {
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::vertical(20, 0),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(20, 0, 60, 24))]);
        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0, 0);
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
    }

    /// The column case of the rule below, against the minimum's `cols` rather than its rows.
    #[test]
    fn a_terminal_too_narrow_for_a_vertical_band_applies_none_of_it() {
        let reserved = Reserved::vertical(20, 0);
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 21 },
            reserved,
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.applied_bands(), Reserved::default());
        assert_eq!(layout.content, Size { rows: 24, cols: 21 });

        layout.resurface(Size { rows: 24, cols: 22 }, reserved, 0, 0);
        assert_eq!(layout.applied_bands(), reserved);
        assert_eq!(layout.content, Size { rows: 24, cols: 2 });
    }

    /// All-or-nothing is per axis. A terminal short of rows still affords the columns, and
    /// hiding the sidebar over a shortage of rows would give the panes nothing back for it.
    #[test]
    fn an_axis_that_fits_keeps_its_bands_when_the_other_does_not() {
        let mut reserved = Reserved::horizontal(0, 4);
        reserved.add(Edge::Left, 20);
        let layout = PaneLayout::new(
            Size { rows: 5, cols: 80 },
            reserved,
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(
            layout.applied_bands(),
            Reserved::vertical(20, 0),
            "the columns fit, so they were taken; the rows did not, so they were not"
        );
        assert_eq!(layout.content, Size { rows: 5, cols: 60 });

        let layout = PaneLayout::new(
            Size { rows: 24, cols: 21 },
            reserved,
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(
            layout.applied_bands(),
            Reserved::horizontal(0, 4),
            "and the other way round"
        );
        assert_eq!(layout.content, Size { rows: 20, cols: 21 });
    }

    /// The reservation is all or nothing: a surface that cannot leave the panes a grid at
    /// least the minimum the caller gave applies none of it, rather than honouring part of a
    /// band its owner would then draw outside.
    #[test]
    fn a_terminal_too_small_for_the_bands_applies_none_of_them() {
        let reserved = bottom(1);
        let min = Size { rows: 2, cols: 2 };
        let mut layout = PaneLayout::new(min, reserved, 0, 0, pane(1), min);
        assert_eq!(layout.content, min);
        assert_eq!(layout.applied_bands(), Reserved::default());

        layout.resurface(
            Size {
                rows: min.rows + 1,
                cols: min.cols,
            },
            reserved,
            0,
            0,
        );
        assert_eq!(layout.content, min, "growing back brings the band back");
        assert_eq!(layout.applied_bands(), reserved);
    }

    #[test]
    fn releasing_a_reservation_gives_the_panes_their_rows_back() {
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            bottom(1),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 23))]);
        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0, 0);
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
    }

    /// A band is outside every pane's rect, so nothing a pane's child writes can land in it.
    #[test]
    fn no_pane_rect_overlaps_a_reserved_band() {
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::horizontal(1, 1),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
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
        layout.resurface(Size { rows: 24, cols: 3 }, Reserved::default(), 0, 0);

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

        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0, 0);
        assert_eq!(layout.showable().count(), 2, "growing back brings it back");
    }

    /// The gutter is the host reserving space on a plugin's behalf, exactly as a band at the
    /// edge is: the panes lose the cells and never learn what goes in them.
    #[test]
    fn a_gutter_costs_the_panes_the_cells_between_them() {
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::default(),
            1,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(41, 0, 39, 24)),
            ]
        );

        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0, 0);
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
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 4 },
            Reserved::default(),
            1,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        assert_eq!(
            layout.split(SplitDir::Vertical, pane(2)),
            Err(SplitRefused::TooSmall),
            "four columns hold two panes of two, but not with a gutter between them"
        );
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(0, 0, 4, 24))]);
    }

    #[test]
    fn the_epoch_names_the_content_size_the_panes_and_the_focus() {
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            bottom(1),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert!(layout.epoch().matches(&Epoch::new(
            Size { rows: 23, cols: 80 },
            vec![pane(1), pane(2)],
            Some(pane(2)),
        )));
    }

    /// The minimum is the caller's (design §"Rounding and minimum sizes"), so a caller that
    /// says something other than `MIN_GRID` gets what it said. Nothing in this file knows
    /// what `vt100` can survive.
    #[test]
    fn the_minimum_a_caller_gives_is_the_minimum_that_is_used() {
        let big = Size { rows: 10, cols: 10 };
        let reserved = bottom(1);
        let layout = PaneLayout::new(Size { rows: 10, cols: 80 }, reserved, 0, 0, pane(1), big);
        assert_eq!(
            layout.applied_bands(),
            Reserved::default(),
            "ten rows cannot hold a band and a ten-row minimum"
        );

        let small = Size { rows: 2, cols: 2 };
        let layout = PaneLayout::new(Size { rows: 10, cols: 80 }, reserved, 0, 0, pane(1), small);
        assert_eq!(
            layout.applied_bands(),
            reserved,
            "the same terminal holds the band when the caller asks for less"
        );
    }

    fn two_side_by_side() -> PaneLayout {
        let mut layout = layout(24, 80);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        layout
    }

    /// Zooming gives the focused pane the whole area and shows nothing else, while the tree
    /// and every pane in it stay: unzooming puts the rects back exactly as they were.
    #[test]
    fn a_zoomed_pane_is_shown_alone_over_the_whole_area_and_comes_back_where_it_was() {
        let mut layout = two_side_by_side();
        let before = layout.panes();

        assert!(layout.toggle_zoom());
        assert_eq!(layout.zoomed(), Some(pane(2)));
        assert_eq!(layout.panes(), vec![(pane(2), layout.area())]);
        assert_eq!(
            layout.showable().collect::<Vec<_>>(),
            vec![(pane(2), layout.area())]
        );
        assert_eq!(layout.shown.len(), 1);
        assert_eq!(
            layout.arranged(),
            vec![pane(1), pane(2)],
            "the hidden pane is still the window's"
        );
        assert_eq!(layout.rects.len(), 2);

        assert!(!layout.toggle_zoom());
        assert_eq!(layout.panes(), before);
        assert_eq!(layout.shown, layout.rects);
    }

    #[test]
    fn a_window_of_one_pane_has_nothing_to_zoom() {
        let mut layout = layout(24, 80);
        assert!(!layout.toggle_zoom());
        assert_eq!(layout.zoomed(), None);
    }

    /// tmux's rules: anything that needs the other panes seen — focusing one, splitting,
    /// moving a boundary — unzooms first.
    #[test]
    fn focusing_a_hidden_pane_splitting_or_resizing_unzooms() {
        let mut layout = two_side_by_side();
        layout.toggle_zoom();
        assert!(layout.set_focus(pane(2)), "the zoomed pane itself");
        assert_eq!(layout.zoomed(), Some(pane(2)));
        assert!(layout.set_focus(pane(1)));
        assert_eq!(layout.zoomed(), None);
        assert_eq!(layout.shown.len(), 2);

        layout.toggle_zoom();
        layout.split(SplitDir::Horizontal, pane(3)).expect("fits");
        assert_eq!(layout.zoomed(), None);

        layout.toggle_zoom();
        assert!(
            layout.resize(Dir::Left, 1).is_err(),
            "the left column has nothing to its left"
        );
        assert_eq!(
            layout.zoomed(),
            Some(pane(3)),
            "a refused resize changes nothing"
        );
        layout.resize(Dir::Right, 1).expect("room");
        assert_eq!(layout.zoomed(), None);
    }

    /// The zoomed pane closing takes the zoom with it; another one closing behind it does not.
    #[test]
    fn a_closed_pane_ends_the_zoom_only_if_it_was_the_zoomed_one() {
        let mut layout = two_side_by_side();
        layout.split(SplitDir::Horizontal, pane(3)).expect("fits");
        layout.toggle_zoom();
        assert_eq!(layout.zoomed(), Some(pane(3)));

        layout.close(pane(1));
        assert_eq!(layout.zoomed(), Some(pane(3)));
        assert_eq!(layout.panes(), vec![(pane(3), layout.area())]);

        layout.close(pane(3));
        assert_eq!(layout.zoomed(), None);
        assert_eq!(layout.panes().len(), 1);
    }

    /// A terminal resize keeps the zoom, over the new area.
    #[test]
    fn a_zoom_survives_the_surface_changing() {
        let mut layout = two_side_by_side();
        layout.toggle_zoom();
        layout.resurface(
            Size {
                rows: 30,
                cols: 100,
            },
            Reserved::default(),
            0,
            0,
        );
        assert_eq!(layout.panes(), vec![(pane(2), PRect::new(0, 0, 100, 30))]);
    }

    fn banded(rows: u16, cols: u16, band: u16) -> PaneLayout {
        PaneLayout::new(
            Size { rows, cols },
            Reserved::default(),
            0,
            band,
            pane(1),
            Size { rows: 2, cols: 2 },
        )
    }

    /// The band is taken off what the pane's shell gets, and not off where the pane sits: the
    /// outer rect is what the tree cut, and the content rect is the outer one less the band.
    #[test]
    fn a_pane_band_takes_its_columns_off_the_content_and_not_the_outer_rect() {
        let layout = banded(24, 80, 10);
        assert_eq!(
            layout.outer_panes(),
            vec![(pane(1), PRect::new(0, 0, 80, 24))]
        );
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(10, 0, 70, 24))]);
        assert_eq!(layout.rects[&pane(1)], PRect::new(10, 0, 70, 24));
        assert_eq!(
            layout.showable().collect::<Vec<_>>(),
            vec![(pane(1), PRect::new(10, 0, 70, 24))]
        );
    }

    #[test]
    fn every_pane_of_a_split_gets_its_own_band() {
        let mut layout = banded(24, 80, 10);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(
            layout.outer_panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(40, 0, 40, 24)),
            ]
        );
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(10, 0, 30, 24)),
                (pane(2), PRect::new(50, 0, 30, 24)),
            ]
        );
    }

    /// One pane the terminal has shrunk below the minimum plus the band loses its band for the
    /// frame; the panes that still have room keep theirs.
    #[test]
    fn a_pane_too_narrow_for_its_band_keeps_its_outer_rect_and_the_others_keep_theirs() {
        let mut layout = banded(24, 80, 10);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        layout.split(SplitDir::Vertical, pane(3)).expect("fits");
        layout.resurface(Size { rows: 24, cols: 40 }, Reserved::default(), 0, 10);
        assert_eq!(
            layout.outer_panes(),
            vec![
                (pane(1), PRect::new(0, 0, 20, 24)),
                (pane(2), PRect::new(20, 0, 10, 24)),
                (pane(3), PRect::new(30, 0, 10, 24)),
            ]
        );
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(10, 0, 10, 24)),
                (pane(2), PRect::new(20, 0, 10, 24)),
                (pane(3), PRect::new(30, 0, 10, 24)),
            ]
        );
    }

    #[test]
    fn a_split_that_would_leave_a_pane_no_room_for_its_band_is_refused() {
        let mut with_band = banded(24, 20, 10);
        assert!(with_band.split(SplitDir::Vertical, pane(2)).is_err());
        assert_eq!(with_band.panes().len(), 1);

        let mut plain = layout(24, 20);
        assert!(
            plain.split(SplitDir::Vertical, pane(2)).is_ok(),
            "the same split fits with no band"
        );
    }

    #[test]
    fn a_zoomed_pane_has_its_band_over_the_whole_area() {
        let mut layout = banded(24, 80, 10);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        layout.toggle_zoom();
        assert_eq!(layout.outer_panes(), vec![(pane(2), layout.area())]);
        assert_eq!(layout.panes(), vec![(pane(2), PRect::new(10, 0, 70, 24))]);

        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0, 0);
        assert_eq!(layout.panes(), vec![(pane(2), layout.area())]);
        layout.toggle_zoom();
        assert_eq!(layout.panes(), layout.outer_panes());
    }
}

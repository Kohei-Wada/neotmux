use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_proto::{Axis, Edge, Rect, Reserved, SessionId, Size};

use crate::draw::ReservationId;
use crate::plugin::SourceId;

/// The most rows one horizontal edge can be reserved for, whoever asks. A band there is a
/// line or two of text along an edge — a status line, a window list — and past four the
/// number is not a band but a mistake (a pixel count, a colour), refused for the same reason
/// `neotmux.opt.pane_gutter` is bounded.
pub const MAX_RESERVED_ROWS: u16 = 4;

/// The most columns one vertical edge can be reserved for. Not four: the bound is not a
/// number of lines of text but a share of the surface, and a terminal has far more columns
/// than rows. Half of the 80 columns everything in a terminal is still written against is
/// the line — a sidebar wide enough for a window name and a command, and past it the band
/// is not a margin on the panes but the layout.
pub const MAX_RESERVED_COLS: u16 = 40;

/// The bound one edge is refused against, in the edge's own unit.
///
/// It is a constant rather than a measurement of the terminal because `init.lua` runs before
/// there is a session: at the moment a plugin asks, there is no surface to measure. The
/// terminal's own limit is a separate judgement made every frame by whoever owns the layout,
/// because a terminal can shrink long after the reservation was granted (§6).
pub fn max_reserved(axis: Axis) -> u16 {
    match axis {
        Axis::Rows => MAX_RESERVED_ROWS,
        Axis::Columns => MAX_RESERVED_COLS,
    }
}

struct Entry {
    id: ReservationId,
    owner: SourceId,
    edge: Edge,
    /// In the edge's own unit: rows for a horizontal band, columns for a vertical one.
    cells: u16,
}

#[derive(Default)]
struct Inner {
    next: u64,
    /// In grant order, which — since a reservation is asked for while its chunk loads — is
    /// load order. An edge is filled from the outside in along it.
    entries: Vec<Entry>,
    /// Where each source draws in each session, whose surfaces are their own terminals'.
    /// A session's map is empty while its surface cannot afford the bands, and absent until
    /// its first frame lays them out.
    bands: BTreeMap<SessionId, BTreeMap<SourceId, Rect>>,
}

impl Inner {
    /// What `edge` is already spending, counting every band there but `except`.
    fn taken_on(&self, edge: Edge, except: Option<ReservationId>) -> u16 {
        self.entries
            .iter()
            .filter(|entry| entry.edge == edge && Some(entry.id) != except)
            .map(|entry| entry.cells)
            .sum()
    }
}

/// Who has asked for a band of the surface, and where each one sits this frame.
#[derive(Default)]
pub struct Reservations {
    inner: RefCell<Inner>,
}

impl Reservations {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Err` carries the message handed back to the caller; nothing is changed in that case.
    pub fn add(&self, owner: SourceId, edge: Edge, cells: u16) -> Result<ReservationId, String> {
        let mut inner = self.inner.borrow_mut();
        if inner.entries.iter().any(|entry| entry.owner == owner) {
            return Err(
                "already holds a reservation; remove it before asking for another".to_string(),
            );
        }
        let axis = edge.axis();
        let cap = max_reserved(axis);
        let taken = inner.taken_on(edge, None);
        if taken + cells > cap {
            return Err(format!(
                "the {edge} edge has {taken} of its {cap} {axis} reserved, \
                 so {cells} more will not fit"
            ));
        }
        inner.next += 1;
        let id = ReservationId(inner.next);
        inner.entries.push(Entry {
            id,
            owner,
            edge,
            cells,
        });
        Ok(id)
    }

    /// A granted band takes a different number of cells of the same edge, from the next frame
    /// on. Its place among the bands on that edge is its place in grant order, and that does
    /// not move — so a band that shrinks and grows again comes back where it was, rather than
    /// walking inside whoever reserved after it the way a remove-and-reserve would.
    ///
    /// `Err` carries the message handed back to the caller; nothing is changed in that case.
    pub fn resize(&self, id: ReservationId, cells: u16) -> Result<(), String> {
        let mut inner = self.inner.borrow_mut();
        let Some(edge) = inner
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.edge)
        else {
            return Err("the reservation has already been removed".to_string());
        };
        let axis = edge.axis();
        let cap = max_reserved(axis);
        let taken = inner.taken_on(edge, Some(id));
        if taken + cells > cap {
            return Err(format!(
                "the {edge} edge has {taken} of its {cap} {axis} reserved by others, \
                 so {cells} will not fit"
            ));
        }
        for entry in inner.entries.iter_mut() {
            if entry.id == id {
                entry.cells = cells;
            }
        }
        Ok(())
    }

    pub fn remove(&self, id: ReservationId) -> bool {
        let mut inner = self.inner.borrow_mut();
        let before = inner.entries.len();
        inner.entries.retain(|entry| entry.id != id);
        before != inner.entries.len()
    }

    pub fn remove_all_of(&self, owner: SourceId) -> usize {
        let mut inner = self.inner.borrow_mut();
        let before = inner.entries.len();
        inner.entries.retain(|entry| entry.owner != owner);
        before - inner.entries.len()
    }

    /// What has been granted so far, per edge. Whether the surface can afford it is not this
    /// module's judgement.
    pub fn totals(&self) -> Reserved {
        let inner = self.inner.borrow();
        let mut reserved = Reserved::default();
        for entry in &inner.entries {
            reserved.add(entry.edge, entry.cells);
        }
        reserved
    }

    /// The host has settled `session`'s surface for this frame: give every granted reservation
    /// its band there in surface coordinates, or — for an axis the surface could not afford, which `applied`
    /// reports as nothing reserved on its edges — no band at all.
    ///
    /// A vertical band spans the full height and a horizontal one only what the vertical
    /// bands left, so no two bands ever share a cell. The corner goes to the sidebar, as
    /// herdr's does: a sidebar is a column down the terminal, and a status line is a line
    /// across the panes beside it.
    ///
    /// Called at a frame boundary and nowhere else, so the bands a tick reads are the ones
    /// the layout it is drawing against was computed from. Every other session keeps its own.
    pub fn lay_out(&self, session: SessionId, surface: Size, applied: Reserved) {
        let mut inner = self.inner.borrow_mut();
        let mut bands = BTreeMap::new();
        let middle_x = applied.left;
        let middle_w = surface.cols.saturating_sub(applied.cols());
        let (mut top, mut bottom, mut left, mut right) = (0u16, 0u16, 0u16, 0u16);
        for entry in &inner.entries {
            if applied.of(entry.edge) == 0 {
                continue;
            }
            let rect = match entry.edge {
                Edge::Top => {
                    let rect = Rect::new(middle_x, top, middle_w, entry.cells);
                    top += entry.cells;
                    rect
                }
                Edge::Bottom => {
                    bottom += entry.cells;
                    Rect::new(
                        middle_x,
                        surface.rows.saturating_sub(bottom),
                        middle_w,
                        entry.cells,
                    )
                }
                Edge::Left => {
                    let rect = Rect::new(left, 0, entry.cells, surface.rows);
                    left += entry.cells;
                    rect
                }
                Edge::Right => {
                    right += entry.cells;
                    Rect::new(
                        surface.cols.saturating_sub(right),
                        0,
                        entry.cells,
                        surface.rows,
                    )
                }
            };
            bands.insert(entry.owner, rect);
        }
        inner.bands.insert(session, bands);
    }

    pub fn band(&self, session: SessionId, owner: SourceId) -> Option<Rect> {
        self.inner
            .borrow()
            .bands
            .get(&session)?
            .get(&owner)
            .copied()
    }

    /// A session is over, and so are the bands it was laid out with.
    pub fn forget(&self, session: SessionId) {
        self.inner.borrow_mut().bands.remove(&session);
    }

    /// Whose band this is, while it exists.
    pub fn owner(&self, id: ReservationId) -> Option<SourceId> {
        self.inner
            .borrow()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.owner)
    }

    /// Who holds each band, in reservation order.
    pub fn owners(&self) -> Vec<SourceId> {
        self.inner
            .borrow()
            .entries
            .iter()
            .map(|entry| entry.owner)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Sources;

    fn sources() -> (Sources, SourceId, SourceId) {
        let sources = Sources::new();
        let a = sources.intern("@a.lua");
        let b = sources.intern("@b.lua");
        (sources, a, b)
    }

    fn surface() -> Size {
        Size { rows: 24, cols: 80 }
    }

    fn session() -> SessionId {
        SessionId::from(1)
    }

    /// The ordinary case: the surface affords everything that was granted.
    fn lay_out(reservations: &Reservations) {
        reservations.lay_out(session(), surface(), reservations.totals());
    }

    #[test]
    fn a_reservation_takes_rows_from_its_own_edge() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.add(b, Edge::Top, 2).expect("granted");
        assert_eq!(reservations.totals(), Reserved::horizontal(2, 1));
    }

    #[test]
    fn two_sources_on_one_edge_get_separate_bands_from_the_outside_in() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.add(b, Edge::Bottom, 1).expect("granted");
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 23, 80, 1))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(0, 22, 80, 1)),
            "the second to ask sits inside the first"
        );
    }

    #[test]
    fn the_top_edge_is_filled_downwards_in_the_same_order() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Top, 2).expect("granted");
        reservations.add(b, Edge::Top, 1).expect("granted");
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 80, 2))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(0, 2, 80, 1))
        );
    }

    #[test]
    fn a_vertical_band_is_columns_off_its_own_edge() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Left, 20).expect("granted");
        reservations.add(b, Edge::Right, 10).expect("granted");
        assert_eq!(reservations.totals(), Reserved::vertical(20, 10));
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 20, 24))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(70, 0, 10, 24))
        );
    }

    #[test]
    fn two_sources_on_one_vertical_edge_get_separate_bands_from_the_outside_in() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Left, 10).expect("granted");
        reservations.add(b, Edge::Left, 6).expect("granted");
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 10, 24))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(10, 0, 6, 24)),
            "the second to ask sits inside the first"
        );
    }

    /// A horizontal band takes only what the vertical ones left, so the corner belongs to
    /// exactly one source — the sidebar, as herdr's does.
    #[test]
    fn a_horizontal_band_starts_where_a_vertical_one_stops() {
        let (s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Top, 1).expect("granted");
        reservations.add(b, Edge::Left, 20).expect("granted");
        reservations
            .add(s.intern("@c.lua"), Edge::Right, 10)
            .expect("granted");
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(20, 0, 50, 1))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(0, 0, 20, 24))
        );
    }

    /// The host refuses the axes separately, and so does the hand-out: the bands of the axis
    /// that fits are still placed, against the surface the other axis was not taken from.
    #[test]
    fn an_axis_the_surface_refused_loses_its_bands_and_the_other_keeps_its_own() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Top, 1).expect("granted");
        reservations.add(b, Edge::Left, 20).expect("granted");
        reservations.lay_out(session(), surface(), Reserved::vertical(20, 0));
        assert_eq!(reservations.band(session(), a), None);
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(0, 0, 20, 24))
        );
        reservations.lay_out(session(), surface(), Reserved::horizontal(1, 0));
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 80, 1))
        );
        assert_eq!(reservations.band(session(), b), None);
    }

    /// Forty columns is a sidebar; forty rows is a mistake. The two bounds are decided
    /// separately, and neither edge spends the other's.
    #[test]
    fn the_vertical_edges_are_bounded_in_columns_not_rows() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations
            .add(a, Edge::Left, MAX_RESERVED_COLS)
            .expect("granted");
        let refused = reservations.add(b, Edge::Left, 1).expect_err("refused");
        assert!(refused.contains("columns reserved"), "{refused}");
        reservations
            .add(b, Edge::Right, MAX_RESERVED_ROWS)
            .expect("a count that fills a horizontal edge is nothing much on a vertical one");
        assert_eq!(
            reservations.totals(),
            Reserved::vertical(MAX_RESERVED_COLS, MAX_RESERVED_ROWS)
        );
    }

    #[test]
    fn a_source_that_reserved_nothing_has_no_band() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        lay_out(&reservations);
        assert_eq!(reservations.band(session(), b), None);
    }

    #[test]
    fn a_surface_that_cannot_afford_the_bands_hands_out_none() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.lay_out(session(), surface(), Reserved::default());
        assert_eq!(reservations.band(session(), a), None);
        lay_out(&reservations);
        assert!(
            reservations.band(session(), a).is_some(),
            "growing back brings it back"
        );
    }

    #[test]
    fn an_edge_past_its_cap_is_refused_and_changes_nothing() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations
            .add(a, Edge::Bottom, MAX_RESERVED_ROWS)
            .expect("granted");
        let refused = reservations.add(b, Edge::Bottom, 1).expect_err("refused");
        assert!(refused.contains("will not fit"), "{refused}");
        assert_eq!(
            reservations.totals(),
            Reserved::horizontal(0, MAX_RESERVED_ROWS)
        );
        lay_out(&reservations);
        assert_eq!(reservations.band(session(), b), None);
    }

    #[test]
    fn the_other_edge_is_unaffected_by_a_full_one() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations
            .add(a, Edge::Bottom, MAX_RESERVED_ROWS)
            .expect("granted");
        reservations.add(b, Edge::Top, 1).expect("granted");
        assert_eq!(
            reservations.totals(),
            Reserved::horizontal(1, MAX_RESERVED_ROWS)
        );
    }

    #[test]
    fn a_second_reservation_from_one_source_is_refused() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        let refused = reservations.add(a, Edge::Top, 1).expect_err("refused");
        assert!(refused.contains("already holds a reservation"), "{refused}");
        assert_eq!(reservations.totals(), Reserved::horizontal(0, 1));
    }

    #[test]
    fn removing_a_reservation_gives_its_rows_back() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        let id = reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.add(b, Edge::Bottom, 1).expect("granted");
        assert!(reservations.remove(id));
        assert!(!reservations.remove(id), "remove is idempotent");
        assert_eq!(reservations.totals(), Reserved::horizontal(0, 1));

        lay_out(&reservations);
        assert_eq!(reservations.band(session(), a), None);
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(0, 23, 80, 1)),
            "the survivor moves out to the edge the removed one held"
        );
    }

    #[test]
    fn a_source_can_reserve_again_once_it_has_removed_its_own() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        let id = reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.remove(id);
        reservations.add(a, Edge::Top, 1).expect("granted again");
        assert_eq!(reservations.totals(), Reserved::horizontal(1, 0));
    }

    /// The point of resizing rather than re-reserving: the outer band folds and unfolds, and
    /// the one inside it neither moves out nor swallows it.
    #[test]
    fn a_band_that_resizes_keeps_its_place_on_the_edge() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        let outer = reservations.add(a, Edge::Left, 10).expect("granted");
        reservations.add(b, Edge::Left, 6).expect("granted");
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 10, 24))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(10, 0, 6, 24))
        );

        reservations.resize(outer, 1).expect("granted");
        assert_eq!(reservations.totals(), Reserved::vertical(7, 0));
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 1, 24))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(1, 0, 6, 24)),
            "the inner band followed the outer one in, and is still inside it"
        );

        reservations.resize(outer, 10).expect("granted");
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 10, 24))
        );
        assert_eq!(
            reservations.band(session(), b),
            Some(Rect::new(10, 0, 6, 24)),
            "unfolding puts both back exactly where they were"
        );
    }

    #[test]
    fn a_resize_past_the_edges_cap_is_refused_and_changes_nothing() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        let id = reservations.add(a, Edge::Left, 10).expect("granted");
        reservations
            .add(b, Edge::Left, MAX_RESERVED_COLS - 10)
            .expect("granted");
        let refused = reservations
            .resize(id, 11)
            .expect_err("one more than the edge can spare");
        assert!(refused.contains("will not fit"), "{refused}");
        assert_eq!(
            reservations.totals(),
            Reserved::vertical(MAX_RESERVED_COLS, 0)
        );
        lay_out(&reservations);
        assert_eq!(
            reservations.band(session(), a),
            Some(Rect::new(0, 0, 10, 24))
        );
    }

    /// The cap is counted against the other bands only, so a band already at the cap may be
    /// resized to anything up to what it already holds.
    #[test]
    fn a_resize_does_not_pay_for_its_own_cells_twice() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        let id = reservations
            .add(a, Edge::Left, MAX_RESERVED_COLS)
            .expect("granted");
        reservations
            .resize(id, MAX_RESERVED_COLS)
            .expect("the edge is full of nobody but this band");
    }

    #[test]
    fn resizing_a_removed_reservation_is_refused() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        let id = reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.remove(id);
        let refused = reservations.resize(id, 2).expect_err("refused");
        assert!(refused.contains("already been removed"), "{refused}");
        assert_eq!(reservations.totals(), Reserved::default());
    }

    #[test]
    fn dropping_a_source_drops_its_reservation() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        assert_eq!(reservations.remove_all_of(a), 1);
        assert_eq!(reservations.totals(), Reserved::default());
        assert_eq!(reservations.remove_all_of(a), 0);
    }
}

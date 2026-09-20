use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_core::proto::{Edge, Rect, Reserved, Size};

use crate::source::SourceId;

/// The most rows one edge can be reserved for, whoever asks. A band is a line or two of text
/// along an edge — a status line, a window list — and past four the number is not a band but
/// a mistake (a pixel count, a colour), refused for the same reason `neotmux.opt.pane_gutter`
/// is bounded.
///
/// This is the bound a reservation is *refused* against, and it is a constant rather than a
/// measurement of the terminal because `init.lua` runs before there is a session: at the
/// moment a plugin asks, there is no surface to measure. The terminal's own limit is a
/// separate judgement made every frame by whoever owns the layout, because a terminal can
/// shrink long after the reservation was granted (§6).
pub const MAX_RESERVED_ROWS: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReservationId(u64);

struct Entry {
    id: ReservationId,
    owner: SourceId,
    edge: Edge,
    rows: u16,
}

#[derive(Default)]
struct Inner {
    next: u64,
    /// In grant order, which — since a reservation is asked for while its chunk loads — is
    /// load order. An edge is filled from the outside in along it.
    entries: Vec<Entry>,
    /// Where each source draws this frame. Empty while the surface cannot afford the bands.
    bands: BTreeMap<SourceId, Rect>,
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
    pub fn add(&self, owner: SourceId, edge: Edge, rows: u16) -> Result<ReservationId, String> {
        let mut inner = self.inner.borrow_mut();
        if inner.entries.iter().any(|entry| entry.owner == owner) {
            return Err(
                "already holds a reservation; remove it before asking for another".to_string(),
            );
        }
        let taken: u16 = inner
            .entries
            .iter()
            .filter(|entry| entry.edge == edge)
            .map(|entry| entry.rows)
            .sum();
        if taken + rows > MAX_RESERVED_ROWS {
            return Err(format!(
                "the {edge} edge has {taken} of its {MAX_RESERVED_ROWS} rows reserved, \
                 so {rows} more will not fit"
            ));
        }
        inner.next += 1;
        let id = ReservationId(inner.next);
        inner.entries.push(Entry {
            id,
            owner,
            edge,
            rows,
        });
        Ok(id)
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

    /// The rows granted so far, per edge. Whether the surface can afford them is not this
    /// module's judgement.
    pub fn totals(&self) -> Reserved {
        let inner = self.inner.borrow();
        let mut reserved = Reserved::default();
        for entry in &inner.entries {
            reserved.add(entry.edge, entry.rows);
        }
        reserved
    }

    /// The host has settled this frame's surface: give every granted reservation its band in
    /// surface coordinates, or — when `applied` is false, because the surface could not
    /// afford them — no band at all.
    ///
    /// Called at a frame boundary and nowhere else, so the bands a tick reads are the ones
    /// the layout it is drawing against was computed from.
    pub fn lay_out(&self, surface: Size, applied: bool) {
        let mut inner = self.inner.borrow_mut();
        let mut bands = BTreeMap::new();
        if applied {
            let (mut top, mut bottom) = (0u16, 0u16);
            for entry in &inner.entries {
                let rect = match entry.edge {
                    Edge::Top => {
                        let rect = Rect::new(0, top, surface.cols, entry.rows);
                        top += entry.rows;
                        rect
                    }
                    Edge::Bottom => {
                        bottom += entry.rows;
                        Rect::new(
                            0,
                            surface.rows.saturating_sub(bottom),
                            surface.cols,
                            entry.rows,
                        )
                    }
                };
                bands.insert(entry.owner, rect);
            }
        }
        inner.bands = bands;
    }

    pub fn band(&self, owner: SourceId) -> Option<Rect> {
        self.inner.borrow().bands.get(&owner).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Sources;

    fn sources() -> (Sources, SourceId, SourceId) {
        let sources = Sources::new();
        let a = sources.intern("@a.lua");
        let b = sources.intern("@b.lua");
        (sources, a, b)
    }

    fn surface() -> Size {
        Size { rows: 24, cols: 80 }
    }

    #[test]
    fn a_reservation_takes_rows_from_its_own_edge() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.add(b, Edge::Top, 2).expect("granted");
        assert_eq!(reservations.totals(), Reserved::new(2, 1));
    }

    #[test]
    fn two_sources_on_one_edge_get_separate_bands_from_the_outside_in() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.add(b, Edge::Bottom, 1).expect("granted");
        reservations.lay_out(surface(), true);
        assert_eq!(reservations.band(a), Some(Rect::new(0, 23, 80, 1)));
        assert_eq!(
            reservations.band(b),
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
        reservations.lay_out(surface(), true);
        assert_eq!(reservations.band(a), Some(Rect::new(0, 0, 80, 2)));
        assert_eq!(reservations.band(b), Some(Rect::new(0, 2, 80, 1)));
    }

    #[test]
    fn a_source_that_reserved_nothing_has_no_band() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.lay_out(surface(), true);
        assert_eq!(reservations.band(b), None);
    }

    #[test]
    fn a_surface_that_cannot_afford_the_bands_hands_out_none() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.lay_out(surface(), false);
        assert_eq!(reservations.band(a), None);
        reservations.lay_out(surface(), true);
        assert!(
            reservations.band(a).is_some(),
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
        assert_eq!(reservations.totals(), Reserved::new(0, MAX_RESERVED_ROWS));
        reservations.lay_out(surface(), true);
        assert_eq!(reservations.band(b), None);
    }

    #[test]
    fn the_other_edge_is_unaffected_by_a_full_one() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        reservations
            .add(a, Edge::Bottom, MAX_RESERVED_ROWS)
            .expect("granted");
        reservations.add(b, Edge::Top, 1).expect("granted");
        assert_eq!(reservations.totals(), Reserved::new(1, MAX_RESERVED_ROWS));
    }

    #[test]
    fn a_second_reservation_from_one_source_is_refused() {
        let (_s, a, _b) = sources();
        let reservations = Reservations::new();
        reservations.add(a, Edge::Bottom, 1).expect("granted");
        let refused = reservations.add(a, Edge::Top, 1).expect_err("refused");
        assert!(refused.contains("already holds a reservation"), "{refused}");
        assert_eq!(reservations.totals(), Reserved::new(0, 1));
    }

    #[test]
    fn removing_a_reservation_gives_its_rows_back() {
        let (_s, a, b) = sources();
        let reservations = Reservations::new();
        let id = reservations.add(a, Edge::Bottom, 1).expect("granted");
        reservations.add(b, Edge::Bottom, 1).expect("granted");
        assert!(reservations.remove(id));
        assert!(!reservations.remove(id), "remove is idempotent");
        assert_eq!(reservations.totals(), Reserved::new(0, 1));

        reservations.lay_out(surface(), true);
        assert_eq!(reservations.band(a), None);
        assert_eq!(
            reservations.band(b),
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
        assert_eq!(reservations.totals(), Reserved::new(1, 0));
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

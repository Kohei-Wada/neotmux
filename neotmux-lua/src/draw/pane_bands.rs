use std::cell::RefCell;

use neotmux_proto::Rect;

use crate::draw::ReservationId;
use crate::plugin::SourceId;

/// The most columns every pane band together may take off a pane. A pane band is a gutter of
/// text down a pane's edge — a time, a line number — and past this it is not a gutter but the
/// pane.
pub const MAX_PANE_BAND_COLS: u16 = 24;

struct Entry {
    id: ReservationId,
    owner: SourceId,
    cells: u16,
}

#[derive(Default)]
struct Inner {
    next: u64,
    /// In grant order, which is load order: a pane's bands are filled from its left edge in.
    entries: Vec<Entry>,
}

impl Inner {
    fn taken(&self, except: Option<ReservationId>) -> u16 {
        self.entries
            .iter()
            .filter(|entry| Some(entry.id) != except)
            .map(|entry| entry.cells)
            .sum()
    }
}

/// Who has asked for columns down the left of every pane, and how many. `Reservations` one
/// level down: the host takes the total off every pane it can spare it from, and each source
/// is handed its own columns of what the host took.
#[derive(Default)]
pub struct PaneBands {
    inner: RefCell<Inner>,
}

impl PaneBands {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Err` carries the message handed back to the caller; nothing is changed in that case.
    pub fn add(&self, owner: SourceId, cells: u16) -> Result<ReservationId, String> {
        let mut inner = self.inner.borrow_mut();
        if inner.entries.iter().any(|entry| entry.owner == owner) {
            return Err(
                "already holds a pane band; remove it before asking for another".to_string(),
            );
        }
        let taken = inner.taken(None);
        if taken + cells > MAX_PANE_BAND_COLS {
            return Err(format!(
                "every pane has {taken} of its {MAX_PANE_BAND_COLS} band columns reserved, \
                 so {cells} more will not fit"
            ));
        }
        inner.next += 1;
        let id = ReservationId(inner.next);
        inner.entries.push(Entry { id, owner, cells });
        Ok(id)
    }

    /// A granted band takes a different number of columns from the next frame on, keeping its
    /// place in grant order.
    pub fn resize(&self, id: ReservationId, cells: u16) -> Result<(), String> {
        let mut inner = self.inner.borrow_mut();
        if !inner.entries.iter().any(|entry| entry.id == id) {
            return Err("the pane band has already been removed".to_string());
        }
        let taken = inner.taken(Some(id));
        if taken + cells > MAX_PANE_BAND_COLS {
            return Err(format!(
                "every pane has {taken} of its {MAX_PANE_BAND_COLS} band columns reserved by \
                 others, so {cells} will not fit"
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

    /// The columns every pane is asked to give up, summed over every source.
    pub fn total(&self) -> u16 {
        self.inner.borrow().taken(None)
    }

    /// `owner`'s columns of a pane whose rect is `outer` with its bands and `content` without
    /// them, or `None` when `owner` holds none, the pane did not get its bands this frame, or
    /// the band would reach past what the host took — one granted since the host last laid
    /// out, which it has not made room for yet.
    pub fn band(&self, owner: SourceId, outer: Rect, content: Rect) -> Option<Rect> {
        if content.w >= outer.w {
            return None;
        }
        let inner = self.inner.borrow();
        let mut x = outer.x;
        for entry in &inner.entries {
            let end = x.saturating_add(entry.cells);
            if entry.owner == owner {
                return (end <= content.x).then(|| Rect::new(x, outer.y, entry.cells, outer.h));
            }
            x = end;
        }
        None
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

    fn outer() -> Rect {
        Rect::new(20, 1, 40, 10)
    }

    #[test]
    fn bands_are_handed_out_from_the_pane_edge_in_grant_order() {
        let (_s, a, b) = sources();
        let bands = PaneBands::new();
        bands.add(a, 3).expect("granted");
        bands.add(b, 4).expect("granted");
        assert_eq!(bands.total(), 7);
        let content = Rect::new(27, 1, 33, 10);
        assert_eq!(
            bands.band(a, outer(), content),
            Some(Rect::new(20, 1, 3, 10))
        );
        assert_eq!(
            bands.band(b, outer(), content),
            Some(Rect::new(23, 1, 4, 10))
        );
    }

    #[test]
    fn a_pane_without_its_bands_hands_none_out() {
        let (_s, a, _b) = sources();
        let bands = PaneBands::new();
        bands.add(a, 3).expect("granted");
        assert_eq!(bands.band(a, outer(), outer()), None);
    }

    #[test]
    fn the_cap_counts_every_source_and_resize_keeps_the_order() {
        let (_s, a, b) = sources();
        let bands = PaneBands::new();
        let first = bands.add(a, 10).expect("granted");
        assert!(bands.add(b, 15).is_err());
        bands.add(b, 6).expect("granted");
        assert!(bands.resize(first, 19).is_err());
        bands.resize(first, 2).expect("fits");
        let content = Rect::new(28, 1, 32, 10);
        assert_eq!(
            bands.band(b, outer(), content),
            Some(Rect::new(22, 1, 6, 10))
        );
    }

    #[test]
    fn one_band_per_source() {
        let (_s, a, _b) = sources();
        let bands = PaneBands::new();
        let id = bands.add(a, 1).expect("granted");
        assert!(bands.add(a, 1).is_err());
        assert!(bands.remove(id));
        bands.add(a, 1).expect("granted after the first went");
    }
}

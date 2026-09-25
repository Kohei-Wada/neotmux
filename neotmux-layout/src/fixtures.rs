//! Test helpers shared by `tree.rs`'s and `assign.rs`'s tests.

use neotmux_proto::{PaneId, Rect, Size};

use crate::{Tree, assign};

pub(crate) const MIN: Size = Size { rows: 2, cols: 2 };

pub(crate) fn pane(id: u32) -> PaneId {
    PaneId::from(id)
}

pub(crate) const SIZES: [(u16, u16); 12] = [
    (1, 1),
    (2, 2),
    (3, 2),
    (2, 3),
    (5, 7),
    (7, 5),
    (9, 9),
    (80, 24),
    (81, 25),
    (13, 60),
    (199, 59),
    (200, 60),
];

/// The gutters the property is checked at: none, one (what a divider costs), and two.
pub(crate) const GUTTERS: [u16; 3] = [0, 1, 2];

/// The property itself, in one place. A gutter means the rects no longer tile `surface`,
/// so what is asserted at every size and every gutter is:
///
/// - one rect per pane, and none of them outside the surface;
/// - no two rects overlap;
/// - any two non-empty rects are at least `gutter` cells apart, which is the clause that
///   says no rect took a gutter cell for itself;
/// - every cell of the surface is covered, once the rects are grown by `gutter` along the
///   right and bottom edges the gutter is reserved on — that is, the rects and the gutters
///   together, and nothing else, cover the surface.
///
/// At `gutter == 0` the growth is the identity and the separation clause is vacuous, so
/// the four reduce to exactly the old "the rects tile the surface exactly".
pub(crate) fn assert_tiles(tree: &Tree, surface: Rect, gutter: u16, case: &str) {
    let (w, h) = (surface.w, surface.h);
    let rects = assign(tree, surface, gutter);
    assert_eq!(
        rects.len(),
        tree.panes().len(),
        "{case}: every pane gets a rect at {w}x{h} gutter {gutter}"
    );

    let mut covered = vec![false; usize::from(w) * usize::from(h)];
    for (id, r) in &rects {
        assert!(
            r.x + r.w <= surface.x + surface.w && r.y + r.h <= surface.y + surface.h,
            "{case}: {id} at {r:?} leaves the {w}x{h} surface"
        );
        // An empty rect grows too: a split with no room left divides nothing, and the
        // cells its children stand on are the gutter it spent them on.
        let x1 =
            r.x.saturating_add(r.w)
                .saturating_add(gutter)
                .min(surface.x + w);
        let y1 =
            r.y.saturating_add(r.h)
                .saturating_add(gutter)
                .min(surface.y + h);
        for y in r.y..y1 {
            for x in r.x..x1 {
                covered[usize::from(y - surface.y) * usize::from(w) + usize::from(x - surface.x)] =
                    true;
            }
        }
    }
    if let Some(missed) = covered.iter().position(|seen| !seen) {
        let (x, y) = (missed % usize::from(w), missed / usize::from(w));
        panic!(
            "{case}: cell ({x}, {y}) of the {w}x{h} surface is neither a pane nor a gutter \
             cell, gutter {gutter}: {rects:?}"
        );
    }

    for (i, (a_id, a)) in rects.iter().enumerate() {
        for (b_id, b) in &rects[i + 1..] {
            let overlaps = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
            assert!(
                !overlaps,
                "{case}: {a_id} at {a:?} overlaps {b_id} at {b:?}"
            );
            if a.w == 0 || a.h == 0 || b.w == 0 || b.h == 0 {
                continue;
            }
            let apart = |a0: u16, a1: u16, b0: u16, b1: u16| {
                b0.saturating_sub(a1).max(a0.saturating_sub(b1))
            };
            let gap =
                apart(a.x, a.x + a.w, b.x, b.x + b.w).max(apart(a.y, a.y + a.h, b.y, b.y + b.h));
            assert!(
                gap >= gutter,
                "{case}: {a_id} at {a:?} and {b_id} at {b:?} are {gap} cells apart, \
                 less than the gutter of {gutter}"
            );
        }
    }
}

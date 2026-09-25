//! How many lines have fallen out of the front of a pane's history.
//!
//! [`crate::scrollback`] numbers history lines from the oldest one the pane still holds, so
//! every number shifts down by one each time the ring drops a line — and nothing in the
//! reading path can tell that it happened. A plugin that anchored a selection to a history
//! line needs `dropped + index` to have stayed the same number, and needs to be told when it
//! has not.
//!
//! **`vt100` 0.16.2 does not report evictions.** `Callbacks` has no scroll hook,
//! `Screen::set_scrollback` clamps to the count rather than answering with it, and the ring
//! itself is private. But it does leak the number of lines *appended*, and appends minus
//! growth is evictions:
//!
//! `Grid::scroll_up` bumps a non-zero scrollback offset by one for every line it pushes into
//! the history — that is how a scrolled-back view stays on the same content while the child
//! keeps printing. So a feed made with the offset pinned one line off zero comes back with
//! the offset one line further along for each line the child scrolled away, and the count is
//! the difference.
//!
//! **Where that stops being exact.** The bump is clamped to the length of the ring, and the
//! offset cannot be pinned off zero at all while the ring is empty, so the count is exact on
//! one condition: **no single feed appends as many lines as the ring holds.** Under that
//! condition the offset never reaches the clamp, and a feed into an empty ring cannot
//! overflow it, so nothing goes uncounted. Past it the count is low, and nothing here says
//! so.
//!
//! A pane holds 10 000 lines and a feed is one read from its PTY, at most 8 KiB. Ordinary
//! output needs a byte per line, so it cannot get near. A child that sends thousands of
//! `CSI S` scrolls in one chunk can, and then the count is low — though by then every line
//! a plugin could have anchored to is gone anyway, so what is lost is the absolute
//! numbering rather than the ability to notice.
//!

/// What one feed did to a pane's history, in the two numbers `vt100` will answer: the number
/// of lines held, and the scrollback offset — pinned off zero before the feed so the parser
/// counts the appends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Feed {
    pub len_before: usize,
    pub len_after: usize,
    pub offset_before: usize,
    pub offset_after: usize,
}

/// How many lines that feed pushed out of the front of the history.
///
/// The offset moved once per line appended, and the length grew by the appends that did not
/// evict anything, so the difference is the evictions. An offset that did not move — the
/// pane had no history to pin against, or it was on the alternate screen, which keeps none —
/// counts nothing, and a length that shrank rather than grew counts nothing either: both are
/// the ring not being what it was, not lines falling out of it.
pub fn dropped(feed: Feed) -> usize {
    let appended = feed.offset_after.saturating_sub(feed.offset_before);
    let growth = feed.len_after.saturating_sub(feed.len_before);
    appended.saturating_sub(growth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_that_only_grew_dropped_nothing() {
        assert_eq!(
            dropped(Feed {
                len_before: 3,
                len_after: 7,
                offset_before: 1,
                offset_after: 5,
            }),
            0
        );
    }

    #[test]
    fn a_full_ring_drops_one_line_for_every_line_appended() {
        assert_eq!(
            dropped(Feed {
                len_before: 10,
                len_after: 10,
                offset_before: 1,
                offset_after: 5,
            }),
            4
        );
    }

    /// The feed that fills the ring: part of it grew the history and the rest pushed lines
    /// out of the front.
    fn filling() -> Feed {
        Feed {
            len_before: 8,
            len_after: 10,
            offset_before: 1,
            offset_after: 7,
        }
    }

    #[test]
    fn the_feed_that_fills_the_ring_counts_only_the_appends_past_the_end() {
        assert_eq!(dropped(filling()), 4);
    }

    #[test]
    fn a_feed_that_appended_nothing_drops_nothing() {
        assert_eq!(
            dropped(Feed {
                len_before: 10,
                len_after: 10,
                offset_before: 1,
                offset_after: 1,
            }),
            0
        );
    }

    /// The pane had no history to pin an offset against, so there was nothing to count with
    /// — and nothing to evict either, because the ring was not full.
    #[test]
    fn an_offset_that_could_not_be_pinned_counts_nothing() {
        assert_eq!(
            dropped(Feed {
                len_before: 0,
                len_after: 4,
                offset_before: 0,
                offset_after: 0,
            }),
            0
        );
    }

    /// Entering the alternate screen takes the history out of view: `vt100` answers about
    /// the alternate grid, which keeps none. Nothing was evicted.
    #[test]
    fn a_history_that_went_away_rather_than_growing_drops_nothing() {
        assert_eq!(
            dropped(Feed {
                len_before: 500,
                len_after: 0,
                offset_before: 1,
                offset_after: 0,
            }),
            0
        );
    }
}

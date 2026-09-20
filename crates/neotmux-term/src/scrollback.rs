//! The lines above a pane's visible screen.
//!
//! **Reading moves the view and puts it back.** `vt100` 0.16.2 has no read path into its
//! scrollback that does not go through the screen's own position in it: `Screen::cell` — and
//! therefore everything built on it — answers out of `Grid::visible_rows`, which is offset by
//! whatever `Screen::set_scrollback` was last given. So a read here sets that offset, reads
//! the rows, and restores it before returning. Nothing observes the difference: the move, the
//! read and the move back all happen inside one `&mut Screen`, on the single thread that also
//! renders, so no frame can be drawn in between and no other reader exists to see it.
//!
//! **What a line number means.** Line 0 is the oldest line the pane still holds, and numbers
//! increase downwards, so the line directly above the top of the visible screen is `len - 1`.
//! Output the child prints is appended to the end of the history and takes the next number:
//! nothing already numbered is renumbered, and `len` grows. The exception is the ring filling
//! up — a pane holds a fixed number of lines, and once it holds that many each new line drops
//! the oldest and every number shifts down by one.

/// How many lines the pane holds above its visible screen.
///
/// `set_scrollback` clamps what it is given to the number of lines actually held, so asking
/// to scroll further back than can exist is how that number is read out.
pub fn len(screen: &mut vt100::Screen) -> usize {
    let saved = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let len = screen.scrollback();
    screen.set_scrollback(saved);
    len
}

/// Reads history lines `from` up to `from + count`, oldest first, one at a time. A `from`
/// past the end of the history answers nothing rather than raising, the way a pane that
/// closed answers nothing.
///
/// The window `vt100` can show is one screen tall, so a longer read walks the history a
/// screen at a time. `read` is handed the screen with the line it is asked about at row
/// `row`, and the pane's width, because that is all any reader here has needed.
pub fn walk<T>(
    screen: &mut vt100::Screen,
    from: usize,
    count: usize,
    mut read: impl FnMut(&vt100::Screen, u16, u16) -> T,
) -> Vec<T> {
    let saved = screen.scrollback();
    let len = len(screen);
    let (rows, cols) = screen.size();
    let rows = usize::from(rows);
    let end = from.saturating_add(count).min(len);
    let mut out = Vec::new();
    let mut at = from.min(end);
    while at < end {
        // An offset of `len - at` puts history line `at` at the top of the visible window.
        screen.set_scrollback(len - at);
        let take = (end - at).min(rows);
        for row in 0..take {
            let row = u16::try_from(row).expect("a row index within one screen");
            out.push(read(screen, row, cols));
        }
        at += take;
    }
    screen.set_scrollback(saved);
    out
}

/// History lines `from` up to `from + count`, oldest first, as plain text on the same terms
/// as [`crate::lines`].
pub fn lines(
    screen: &mut vt100::Screen,
    from: usize,
    count: usize,
    trailing: crate::lines::Trailing,
) -> Vec<String> {
    walk(screen, from, count, |screen, row, cols| {
        crate::lines::row(screen, row, cols, trailing)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A screen two rows tall, so anything past the second line is history.
    fn parser(scrollback: usize, feed: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(2, 8, scrollback);
        parser.process(feed);
        parser
    }

    fn visible(parser: &vt100::Parser) -> Vec<String> {
        crate::lines::visible(parser.screen(), None, crate::lines::Trailing::Trim)
    }

    fn lines(screen: &mut vt100::Screen, from: usize, count: usize) -> Vec<String> {
        super::lines(screen, from, count, crate::lines::Trailing::Trim)
    }

    #[test]
    fn a_pane_that_has_not_scrolled_has_no_history() {
        let mut parser = parser(100, b"one\r\ntwo");
        assert_eq!(len(parser.screen_mut()), 0);
        assert!(lines(parser.screen_mut(), 0, 10).is_empty());
    }

    #[test]
    fn what_scrolled_off_the_top_is_readable_above_the_screen() {
        let mut parser = parser(100, b"one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(len(parser.screen_mut()), 2);
        assert_eq!(lines(parser.screen_mut(), 0, 10), vec!["one", "two"]);
        assert_eq!(visible(&parser), vec!["three", "four"]);
    }

    /// The property the whole design turns on.
    #[test]
    fn reading_the_history_does_not_move_the_visible_screen() {
        let mut parser = parser(100, b"one\r\ntwo\r\nthree\r\nfour");
        let before = visible(&parser);
        lines(parser.screen_mut(), 0, 10);
        len(parser.screen_mut());
        assert_eq!(visible(&parser), before);
        assert_eq!(parser.screen().scrollback(), 0, "the view was left moved");
    }

    #[test]
    fn a_read_longer_than_one_screen_walks_the_history() {
        let mut parser = parser(100, b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7");
        assert_eq!(len(parser.screen_mut()), 5);
        assert_eq!(
            lines(parser.screen_mut(), 0, 5),
            vec!["1", "2", "3", "4", "5"],
        );
        assert_eq!(visible(&parser), vec!["6", "7"]);
    }

    #[test]
    fn a_read_starts_where_it_was_asked_to_and_stops_after_the_count() {
        let mut parser = parser(100, b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7");
        assert_eq!(lines(parser.screen_mut(), 2, 2), vec!["3", "4"]);
    }

    #[test]
    fn a_read_past_the_end_of_the_history_answers_nothing() {
        let mut parser = parser(100, b"1\r\n2\r\n3\r\n4");
        assert!(lines(parser.screen_mut(), 99, 10).is_empty());
        assert!(lines(parser.screen_mut(), 0, 0).is_empty());
    }

    #[test]
    fn a_count_past_the_end_stops_at_the_end() {
        let mut parser = parser(100, b"1\r\n2\r\n3\r\n4");
        assert_eq!(lines(parser.screen_mut(), 1, usize::MAX), vec!["2"]);
    }

    /// A saved position has to still mean the same line after the child says more.
    #[test]
    fn new_output_does_not_renumber_the_lines_already_there() {
        let mut parser = parser(100, b"1\r\n2\r\n3\r\n4");
        assert_eq!(lines(parser.screen_mut(), 0, 2), vec!["1", "2"]);
        parser.process(b"\r\n5\r\n6\r\n7");
        assert_eq!(lines(parser.screen_mut(), 0, 2), vec!["1", "2"]);
        assert_eq!(len(parser.screen_mut()), 5);
    }

    /// The one case where a number does move, and it is the ring running out.
    #[test]
    fn a_full_ring_drops_the_oldest_line_and_shifts_every_number_down() {
        let mut parser = parser(2, b"1\r\n2\r\n3\r\n4");
        assert_eq!(lines(parser.screen_mut(), 0, 2), vec!["1", "2"]);
        parser.process(b"\r\n5");
        assert_eq!(len(parser.screen_mut()), 2, "the ring holds two");
        assert_eq!(lines(parser.screen_mut(), 0, 2), vec!["2", "3"]);
    }

    #[test]
    fn a_pane_with_no_scrollback_at_all_keeps_nothing() {
        let mut parser = parser(0, b"1\r\n2\r\n3\r\n4");
        assert_eq!(len(parser.screen_mut()), 0);
        assert!(lines(parser.screen_mut(), 0, 10).is_empty());
    }
}

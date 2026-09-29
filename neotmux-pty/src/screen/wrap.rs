//! Whether a row's text runs on into the row below it.
//!
//! `vt100` 0.16.2 keeps the flag per row and hands it out through `Screen::row_wrapped`,
//! which reads the *visible* window — so a history row is reached the same way its text is,
//! by moving the screen's own position in the scrollback and putting it back
//! ([`crate::screen::scrollback`]).
//!
//! What it means is the terminal's, not ours: the row was filled to its last column and the
//! next character went on to the next row. A row the child ended with a newline is not
//! wrapped, and neither is a row the screen was resized under — `Grid::set_size` clears the
//! flag on every row when the column count changes, because the old wrap points no longer
//! describe the new width.

/// Whether each visible row runs on into the next, top to bottom, bounded the way
/// [`crate::screen::lines::visible`] is bounded.
pub fn visible(screen: &vt100::Screen, last: Option<usize>) -> Vec<bool> {
    let (rows, _) = screen.size();
    (crate::screen::lines::first_row(rows, last)..rows)
        .map(|row| screen.row_wrapped(row))
        .collect()
}

/// Whether each history row `from` up to `from + count` runs on into the next, oldest first.
pub fn history(screen: &mut vt100::Screen, from: usize, count: usize) -> Vec<bool> {
    crate::screen::scrollback::walk(screen, from, count, |screen, row, _| {
        screen.row_wrapped(row)
    })
}

#[cfg(test)]
mod tests {
    use neotmux_proto::screen::Trailing;

    use super::*;

    fn parser(rows: u16, cols: u16, scrollback: usize, feed: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(rows, cols, scrollback);
        parser.process(feed);
        parser
    }

    #[test]
    fn a_line_that_ran_off_the_end_is_wrapped_and_the_one_it_ran_into_is_not() {
        let parser = parser(3, 4, 0, b"abcdef");
        assert_eq!(visible(parser.screen(), None), vec![true, false, false]);
    }

    #[test]
    fn a_line_the_child_ended_itself_is_not_wrapped() {
        let parser = parser(2, 8, 0, b"ab\r\ncd");
        assert_eq!(visible(parser.screen(), None), vec![false, false]);
    }

    /// The flag is bounded with the lines it describes, so the two answers line up
    /// index for index.
    #[test]
    fn the_last_n_rows_are_the_bottom_of_the_screen() {
        let parser = parser(3, 4, 0, b"abcdef");
        assert_eq!(visible(parser.screen(), Some(2)), vec![false, false]);
        assert_eq!(
            crate::screen::lines::visible(parser.screen(), Some(2), Trailing::Trim).len(),
            2
        );
    }

    #[test]
    fn a_wrapped_line_that_scrolled_off_is_still_wrapped_in_the_history() {
        let mut parser = parser(2, 4, 100, b"abcdef\r\n\r\nxy");
        assert_eq!(history(parser.screen_mut(), 0, 10), vec![true, false]);
        assert_eq!(
            crate::screen::scrollback::lines(parser.screen_mut(), 0, 10, Trailing::Trim),
            vec!["abcd", "ef"]
        );
    }

    /// Reading the flags must leave the pane where it was, exactly as reading the text does.
    #[test]
    fn reading_the_history_flags_does_not_move_the_visible_screen() {
        let mut parser = parser(2, 4, 100, b"abcdef\r\n\r\nxy");
        history(parser.screen_mut(), 0, 10);
        assert_eq!(parser.screen().scrollback(), 0, "the view was left moved");
    }

    #[test]
    fn a_pane_with_no_history_has_no_flags_to_report() {
        let mut parser = parser(2, 4, 100, b"ab");
        assert!(history(parser.screen_mut(), 0, 10).is_empty());
    }
}

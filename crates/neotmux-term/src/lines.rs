use neotmux_proto::read::Trailing;

/// One row of a terminal as plain text, and the rules a plugin matching a pattern against it
/// can rely on.
///
/// What a cell contributes:
///
/// - a character the child wrote, exactly as it wrote it;
/// - a wide character once, at the column it starts in — its continuation cell adds nothing,
///   so the string is what the pane reads as rather than what it occupies;
/// - a single space for a cell that holds nothing. A cell that was never written and one that
///   was erased are the same cell to `vt100`, so nothing here can tell them apart, and a
///   plugin must not try to.
///
/// Trailing spaces are cut, so an untouched row is the empty string: the blank rest of a line
/// is the terminal's, not something the child said. [`Trailing::Pad`] keeps them instead,
/// because a rectangle drawn over a short line has to know where its columns are.
pub(crate) fn row(screen: &vt100::Screen, row: u16, cols: u16, trailing: Trailing) -> String {
    let mut out = String::new();
    let mut col = 0;
    while col < cols {
        let Some(cell) = screen.cell(row, col) else {
            break;
        };
        // Reachable only if a lead cell was overwritten out from under its continuation; the
        // width below carries past the continuation in every ordinary case.
        if cell.is_wide_continuation() {
            col += 1;
            continue;
        }
        let text = cell.contents();
        if text.is_empty() {
            out.push(' ');
        } else {
            out.push_str(text);
        }
        col += if cell.is_wide() { 2 } else { 1 };
    }
    match trailing {
        Trailing::Trim => out.truncate(out.trim_end().len()),
        // The walk stops short only where a cell is missing; fill what it never reached, so
        // a padded row is the pane's width whichever way the loop ended.
        Trailing::Pad => (col..cols).for_each(|_| out.push(' ')),
    }
    out
}

/// The visible screen as lines, top to bottom, with no escape sequences in them.
///
/// `last` is how many rows from the bottom to take, and it bounds the work rather than the
/// answer: the rows above it are never built. `None` is the whole screen. What is above the
/// screen is [`crate::scrollback`]'s question, not this one.
pub fn visible(screen: &vt100::Screen, last: Option<usize>, trailing: Trailing) -> Vec<String> {
    let (rows, cols) = screen.size();
    (first_row(rows, last)..rows)
        .map(|r| row(screen, r, cols, trailing))
        .collect()
}

/// The top of the window `last` names, counted from the bottom of the screen.
pub(crate) fn first_row(rows: u16, last: Option<usize>) -> u16 {
    match last {
        Some(last) => rows.saturating_sub(u16::try_from(last).unwrap_or(u16::MAX)),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(rows: u16, cols: u16, feed: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(feed);
        parser
    }

    fn lines(rows: u16, cols: u16, feed: &[u8], last: Option<usize>) -> Vec<String> {
        visible(screen(rows, cols, feed).screen(), last, Trailing::Trim)
    }

    fn padded(rows: u16, cols: u16, feed: &[u8]) -> Vec<String> {
        visible(screen(rows, cols, feed).screen(), None, Trailing::Pad)
    }

    #[test]
    fn what_was_written_comes_back_as_lines() {
        assert_eq!(
            lines(3, 10, b"one\r\ntwo\r\nthree", None),
            vec!["one", "two", "three"]
        );
    }

    #[test]
    fn a_line_is_as_wide_as_the_screen_and_wraps_into_the_next_one() {
        assert_eq!(lines(2, 4, b"abcdef", None), vec!["abcd", "ef"]);
    }

    /// The whole point of lines over bytes: a plugin matching a pattern must not have to
    /// strip SGR first.
    #[test]
    fn no_escape_sequence_survives_into_a_line() {
        let lines = lines(
            1,
            20,
            b"\x1b[1;31mred\x1b[0m \x1b[4munderlined\x1b[0m",
            None,
        );
        assert_eq!(lines, vec!["red underlined"]);
    }

    #[test]
    fn a_wide_character_appears_once_at_the_column_it_starts_in() {
        assert_eq!(
            lines(1, 8, "a\u{3042}\u{3044}b".as_bytes(), None),
            vec!["a\u{3042}\u{3044}b"]
        );
    }

    /// Both are an empty cell to `vt100`, so both read as a space; only the trailing run of
    /// them is cut, and it is cut whichever kind it was.
    #[test]
    fn a_blanked_cell_and_one_that_was_never_written_read_the_same() {
        assert_eq!(lines(1, 8, b"ab  cd  ", None), vec!["ab  cd"]);
        assert_eq!(lines(1, 8, b"abXXcd\x1b[1;3H\x1b[2X", None), vec!["ab  cd"]);
        assert_eq!(lines(1, 8, b"", None), vec![""]);
    }

    #[test]
    fn the_last_n_rows_are_the_bottom_of_the_screen() {
        assert_eq!(
            lines(4, 8, b"one\r\ntwo\r\nthree\r\nfour", Some(2)),
            vec!["three", "four"]
        );
    }

    #[test]
    fn asking_for_more_rows_than_there_are_gives_the_whole_screen() {
        assert_eq!(lines(2, 8, b"one\r\ntwo", Some(99)), vec!["one", "two"]);
    }

    #[test]
    fn asking_for_no_rows_gives_nothing() {
        assert!(lines(2, 8, b"one\r\ntwo", Some(0)).is_empty());
    }

    /// The blank rest of a short line is where a rectangle selection's columns are, so it
    /// has to survive the read that a text reader wants it cut from.
    #[test]
    fn padding_keeps_every_row_as_wide_as_the_pane() {
        assert_eq!(padded(2, 6, b"ab\r\ncd"), vec!["ab    ", "cd    "]);
        assert_eq!(padded(1, 6, b""), vec!["      "]);
    }

    /// Columns, not characters: a padded row that holds a wide character is two cells
    /// shorter in `char`s than the pane is wide, and it is the cells that have to line up.
    #[test]
    fn a_padded_row_is_the_panes_width_in_columns() {
        let line = padded(1, 6, "\u{3042}b".as_bytes()).remove(0);
        assert_eq!(line, "\u{3042}b   ");
        assert_eq!(neotmux_proto::width::of_str(&line), 6);
    }

    /// Output that scrolled off the top is gone from the visible screen; reaching it is
    /// `crate::scrollback`'s job, not this one's.
    #[test]
    fn what_scrolled_off_the_top_is_not_in_the_lines() {
        let lines = lines(2, 8, b"one\r\ntwo\r\nthree\r\n", None);
        assert_eq!(lines, vec!["three", ""]);
    }

    /// The property the width rule exists for: whatever the rule says a string is wide,
    /// the grid `vt100` builds from that string is that many columns wide.
    fn grid_width(text: &str) -> usize {
        let mut parser = vt100::Parser::new(1, 200, 0);
        parser.process(text.as_bytes());
        usize::from(parser.screen().cursor_position().1)
    }

    #[test]
    fn what_this_measures_is_where_the_grid_leaves_the_cursor() {
        for text in ["hello", "\u{3042}\u{3044}", "a\u{3042}b", "e\u{0301}x", ""] {
            assert_eq!(
                neotmux_proto::width::of_str(text),
                grid_width(text),
                "measuring {text:?}"
            );
        }
    }
}

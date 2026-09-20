//! How many columns a string takes on a pane's grid.
//!
//! The rule is `vt100` 0.16.2's own, taken from `Screen::text`, because the grid a pane is
//! drawn from is `vt100`'s: a character's width decides whether the cell it lands in is
//! marked wide, and the renderer skips the continuation cell of a wide one. Measuring any
//! other way would let a plugin's columns and the renderer's columns disagree, which is the
//! whole reason this is in the host rather than in Lua.
//!
//! Two consequences worth naming. A zero-width character — a combining mark — adds nothing,
//! because `vt100` appends it to the cell before it rather than giving it one. A control
//! character below U+0100 adds nothing either, because `vt100` refuses to draw one at all;
//! a tab never reaches a cell, it moves the cursor, so it has no width here.

use unicode_width::UnicodeWidthChar as _;

/// How many columns `c` occupies in a pane.
pub fn of_char(c: char) -> usize {
    match c.width() {
        Some(width) => width,
        // `Screen::text`: an unmeasurable character below U+0100 is a control character and
        // is not drawn; anything else it cannot measure is given one column.
        None if u32::from(c) < 256 => 0,
        None => 1,
    }
}

/// How many columns `text` occupies in a pane.
pub fn of_str(text: &str) -> usize {
    text.chars().map(of_char).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole file exists for: whatever this says a string is wide, the
    /// grid `vt100` builds from that string is that many columns wide.
    fn grid_width(text: &str) -> usize {
        let mut parser = vt100::Parser::new(1, 200, 0);
        parser.process(text.as_bytes());
        usize::from(parser.screen().cursor_position().1)
    }

    #[test]
    fn ascii_is_one_column_a_character() {
        assert_eq!(of_str("hello"), 5);
    }

    #[test]
    fn a_wide_character_is_two_columns() {
        assert_eq!(of_str("\u{3042}"), 2);
        assert_eq!(of_str("a\u{3042}b"), 4);
    }

    #[test]
    fn a_combining_mark_adds_nothing() {
        assert_eq!(of_str("e\u{0301}"), 1);
    }

    /// A tab moves the cursor rather than filling a cell, so it is not a column of text.
    #[test]
    fn a_control_character_adds_nothing() {
        assert_eq!(of_str("\t"), 0);
        assert_eq!(of_str("a\tb"), 2);
        assert_eq!(of_str("\x07"), 0);
    }

    #[test]
    fn an_empty_string_is_no_columns_at_all() {
        assert_eq!(of_str(""), 0);
    }

    #[test]
    fn what_this_measures_is_where_the_grid_leaves_the_cursor() {
        for text in ["hello", "\u{3042}\u{3044}", "a\u{3042}b", "e\u{0301}x", ""] {
            assert_eq!(of_str(text), grid_width(text), "measuring {text:?}");
        }
    }
}

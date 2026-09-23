//! How many columns a string takes on a pane's grid.
//!
//! The rule is `vt100` 0.16.2's own, taken from `Screen::text`, because the grid a pane is
//! drawn from is `vt100`'s: a character's width decides whether the cell it lands in is
//! marked wide, and the renderer skips the continuation cell of a wide one. Measuring any
//! other way would let a plugin's columns and the renderer's columns disagree, which is the
//! whole reason this is in the host rather than in Lua — and why it is here, beside the grid
//! it describes, rather than in the crate that happens to own the emulator.
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
        // is not drawn; anything else it cannot measure is given one column. The second arm
        // cannot be reached with `unicode-width` 0.2 — everything it declines to measure is
        // a control below U+0100 — and is kept because this mirrors `vt100`'s own match, and
        // a mirror that drops a case stops being one. `nothing_at_or_above_u0100_is_unmeasurable`
        // is what says so.
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

    /// The arm `of_char` keeps for `vt100`'s sake — an unmeasurable character at U+0100 or
    /// above is given one column — cannot be reached with `unicode-width` 0.2: the 65 code
    /// points it declines to measure are the C0 and C1 controls and DEL, all below U+0100.
    ///
    /// Kept as a test rather than deleted as dead code, because if a later `unicode-width`
    /// does decline one higher up, this fails here rather than silently splitting the rule
    /// between `of_char` and the two callers that wrote `unwrap_or(0)`.
    #[test]
    fn nothing_at_or_above_u0100_is_unmeasurable() {
        use unicode_width::UnicodeWidthChar as _;
        let unmeasurable: Vec<u32> = (0u32..=0x10FFFF)
            .filter_map(char::from_u32)
            .filter(|c| c.width().is_none())
            .map(u32::from)
            .collect();
        assert_eq!(unmeasurable.len(), 65, "{unmeasurable:?}");
        assert!(
            unmeasurable.iter().all(|&c| c < 0x100),
            "the arm is reachable now: {:?}",
            unmeasurable
                .iter()
                .filter(|&&c| c >= 0x100)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_wide_character_is_two_columns_and_a_combining_mark_is_none() {
        assert_eq!(of_char('あ'), 2);
        assert_eq!(of_char('a'), 1);
        assert_eq!(of_char('\u{0301}'), 0); // combining acute
        assert_eq!(of_char('\u{1b}'), 0); // ESC, a control below U+0100
        assert_eq!(of_str("aあ"), 3);
    }
}

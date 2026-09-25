//! The escape sequence that asks the terminal to put something on the system clipboard.
//!
//! OSC 52: `ESC ] 52 ; c ; <base64> BEL`. `c` is the clipboard selection, which is the one
//! a terminal that implements this at all implements. BEL rather than ST terminates it
//! because that is the form `xterm`'s own `Ms` capability is written in, and it is what the
//! terminals that accept only one of the two accept.
//!
//! **Nothing here can be checked.** The sequence goes out and the terminal either acts on it
//! or discards it as an OSC it does not know; there is no reply, and an unknown OSC is not
//! an error. What the caller can be told is in [`crate::osc52::set`]'s only promise: these
//! are the bytes.

use base64::prelude::{BASE64_STANDARD, Engine as _};

/// The bytes that ask the terminal to put `text` on the clipboard.
pub fn set(text: &str) -> Vec<u8> {
    let mut out = Vec::from(&b"\x1b]52;c;"[..]);
    out.extend_from_slice(BASE64_STANDARD.encode(text).as_bytes());
    out.push(0x07);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_base64_between_the_introducer_and_the_bell() {
        assert_eq!(set("hi"), b"\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn an_empty_selection_is_still_a_well_formed_sequence() {
        assert_eq!(set(""), b"\x1b]52;c;\x07");
    }

    /// The reason the payload is encoded at all: a selection can hold anything the pane held,
    /// newlines and escape characters included, and none of it may reach the terminal raw.
    #[test]
    fn nothing_in_the_text_can_escape_the_sequence() {
        let sequence = set("a\nb\x1b]52;c;evil\x07");
        assert_eq!(sequence.iter().filter(|b| **b == 0x1b).count(), 1);
        assert_eq!(sequence.iter().filter(|b| **b == 0x07).count(), 1);
        assert_eq!(sequence.last(), Some(&0x07));
    }

    /// A terminal that reads this back has to get exactly what was yanked.
    #[test]
    fn what_comes_out_of_the_encoding_is_what_went_in() {
        let text = "\u{3042} line\twith\r\nmuch in it";
        let body = &set(text)[b"\x1b]52;c;".len()..];
        let body = &body[..body.len() - 1];
        assert_eq!(BASE64_STANDARD.decode(body).unwrap(), text.as_bytes());
    }
}

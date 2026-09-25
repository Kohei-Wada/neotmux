//! `ESC [ y ; x f` written as `ESC [ y ; x H`, on the way into the parser.
//!
//! HVP (Horizontal and Vertical Position, final byte `f`) and CUP (Cursor Position, final
//! byte `H`) place the cursor at the same row and column; ECMA-48 separates them only by
//! which of them a format effector may be. `vt100` implements CUP and drops HVP, so a program
//! that positions with `f` draws its whole screen as one run of text with no gaps in it —
//! btop does exactly that, and comes out unreadable.
//!
//! The rewrite is one byte and is done here rather than patched into the parser because the
//! parser is a dependency: what belongs to neotmux is the decision that the two sequences mean
//! the same thing, which they do.
//!
//! It runs on the bytes a *child* wrote, which arrive in whatever pieces the pipe gave, so the
//! scanner carries its state between calls. Only a CSI's final byte is ever looked at: an `f`
//! in ordinary text, in an OSC string, or in a DCS payload is never inside a CSI and is never
//! touched.

/// Where the scanner is between one chunk and the next.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum At {
    /// Ordinary bytes.
    #[default]
    Text,
    /// An `ESC` has been seen and the byte after it decides what it introduces.
    Escape,
    /// Inside `ESC [ ... `, before the final byte.
    Csi,
}

#[derive(Default)]
pub struct Hvp {
    at: At,
}

impl Hvp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hands `bytes` to `feed` in as few pieces as it takes, with every CSI whose final byte is
    /// `f` given `H` instead. A chunk with no HVP in it is handed over whole.
    pub fn rewrite(&mut self, bytes: &[u8], mut feed: impl FnMut(&[u8])) {
        let mut from = 0;
        for (at, byte) in bytes.iter().enumerate() {
            match self.at {
                At::Text => {
                    if *byte == 0x1b {
                        self.at = At::Escape;
                    }
                }
                At::Escape => {
                    self.at = match byte {
                        b'[' => At::Csi,
                        // Another `ESC` starts a fresh one; anything else is a short escape
                        // sequence this scanner has no interest in.
                        0x1b => At::Escape,
                        _ => At::Text,
                    };
                }
                At::Csi => match byte {
                    // Parameter and intermediate bytes: still inside the sequence.
                    0x20..=0x3f => {}
                    // The final byte, which ends it. `f` is the one this exists for.
                    0x40..=0x7e => {
                        self.at = At::Text;
                        if *byte == b'f' {
                            feed(&bytes[from..at]);
                            feed(b"H");
                            from = at + 1;
                        }
                    }
                    // A control character inside a CSI is passed through by the parser and
                    // does not end the sequence; an `ESC` abandons it.
                    0x1b => self.at = At::Escape,
                    _ => {}
                },
            }
        }
        if from < bytes.len() {
            feed(&bytes[from..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewritten(chunks: &[&[u8]]) -> Vec<u8> {
        let mut hvp = Hvp::new();
        let mut out = Vec::new();
        for chunk in chunks {
            hvp.rewrite(chunk, |bytes| out.extend_from_slice(bytes));
        }
        out
    }

    #[test]
    fn an_hvp_becomes_a_cup() {
        assert_eq!(rewritten(&[b"\x1b[3;7f"]), b"\x1b[3;7H");
        assert_eq!(rewritten(&[b"\x1b[f"]), b"\x1b[H");
    }

    /// A child's output arrives in whatever pieces the pipe gave, and a sequence can be split
    /// anywhere in it.
    #[test]
    fn a_sequence_split_across_chunks_is_still_rewritten() {
        assert_eq!(rewritten(&[b"\x1b[3", b";7f"]), b"\x1b[3;7H");
        assert_eq!(rewritten(&[b"\x1b", b"[", b"1", b"f"]), b"\x1b[1H");
        assert_eq!(rewritten(&[b"\x1b[3;7", b"f"]), b"\x1b[3;7H");
    }

    /// Everything else is handed over exactly as it came.
    #[test]
    fn nothing_else_is_touched() {
        for bytes in [
            &b"fff"[..],
            b"\x1b[2J",
            b"\x1b[?25h",
            b"\x1b[38;2;1;2;3m",
            // An OSC string with an `f` in it: never inside a CSI, never rewritten.
            b"\x1b]0;file.txt\x07",
            b"\x1bOA",
            b"plain text with f in it",
        ] {
            assert_eq!(
                rewritten(&[bytes]),
                bytes,
                "{:?}",
                String::from_utf8_lossy(bytes)
            );
        }
    }

    /// The one that matters in practice: btop writes a whole screen as moves and runs.
    #[test]
    fn a_run_of_moves_and_text_keeps_its_text() {
        assert_eq!(
            rewritten(&[b"\x1b[1;1fone\x1b[2;1ftwo\x1b[3;1fthree"]),
            b"\x1b[1;1Hone\x1b[2;1Htwo\x1b[3;1Hthree"
        );
    }
}

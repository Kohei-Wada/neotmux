//! Cutting a chunk just before every OSC 133 it holds.
//!
//! A mark is numbered by the line the cursor is on when `vt100` hands it over, and that
//! number is only known between feeds (`crate::screen::evictions`). So each mark goes in at
//! the front of a piece of its own, before the piece has scrolled anything.

const OPEN: &[u8] = b"\x1b]133;";

/// The chunk in pieces, each after the first starting with `ESC ] 133 ;`. A chunk with no
/// mark is one piece, itself.
pub fn split(bytes: &[u8]) -> Vec<&[u8]> {
    let mut pieces = Vec::new();
    let mut from = 0;
    let mut search = 1;
    while search < bytes.len() {
        let Some(found) = bytes[search..]
            .windows(OPEN.len())
            .position(|window| window == OPEN)
        else {
            break;
        };
        let cut = search + found;
        pieces.push(&bytes[from..cut]);
        from = cut;
        search = cut + 1;
    }
    pieces.push(&bytes[from..]);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_with_no_mark_is_one_piece() {
        assert_eq!(split(b"plain"), vec![&b"plain"[..]]);
        assert_eq!(split(b""), vec![&b""[..]]);
    }

    #[test]
    fn each_mark_starts_a_piece() {
        assert_eq!(
            split(b"ab\x1b]133;A\x07cd\x1b]133;B\x07"),
            vec![&b"ab"[..], &b"\x1b]133;A\x07cd"[..], &b"\x1b]133;B\x07"[..]]
        );
    }

    #[test]
    fn a_mark_at_the_front_is_not_cut_off_an_empty_piece() {
        assert_eq!(split(b"\x1b]133;A\x07x"), vec![&b"\x1b]133;A\x07x"[..]]);
    }
}

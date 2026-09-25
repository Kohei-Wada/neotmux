use std::io::Write as _;

use crate::state::TermState;

const ESC: u8 = 0x1b;

/// A query is a short CSI sequence; anything longer than this is something else that merely
/// started like one, and holding on to it would be a slow leak on a child that writes
/// megabytes of parameter bytes.
const MAX_PENDING: usize = 24;

/// A question a child asked its terminal, of the ones this terminal can answer truthfully.
#[derive(Debug, PartialEq, Eq)]
enum Query {
    /// DA1 — "what are you?". Also the barrier programs use to decide that every earlier
    /// query either has been answered or never will be.
    PrimaryAttributes,
    /// DSR 5 — "are you well?".
    Status,
    /// DSR 6 (CPR) — "where is the cursor?".
    CursorPosition,
    /// DECRQM — "is this mode on?", for one private mode.
    Mode(u16),
}

impl Query {
    fn reply(&self, cursor: (u16, u16), out: &mut Vec<u8>) {
        match self {
            // VT100 with the Advanced Video Option, which is what `vt100` emulates: cursor
            // movement, erasure, scroll regions and character attributes. Every richer
            // answer advertises something we do not do — selective erase, sixel, rectangular
            // editing — and a program that believed it would render wrongly.
            Self::PrimaryAttributes => out.extend_from_slice(b"\x1b[?1;2c"),
            // 0 is "ready, no malfunction", and we are.
            Self::Status => out.extend_from_slice(b"\x1b[0n"),
            Self::CursorPosition => {
                let (row, col) = cursor;
                write!(out, "\x1b[{};{}R", row + 1, col + 1).expect("writing to a Vec");
            }
            // 0 is "mode not recognised", which is the truth for every private mode: this
            // terminal keeps no DECRQM-reportable state. 2 would say "recognised, and reset",
            // which invites a program to set the mode and use it.
            Self::Mode(mode) => write!(out, "\x1b[?{mode};0$y").expect("writing to a Vec"),
        }
    }
}

/// Classifies one complete CSI sequence, given its parameter and intermediate bytes and its
/// final byte. Anything not listed is left unanswered, which is itself an answer: the kitty
/// keyboard query `CSI ? u` is deliberately absent, because a reply — even `CSI ? 0 u` —
/// claims the protocol is implemented, and a program would then switch its keys over to an
/// encoding neotmux does not send. Silence is how that query says "no", and DA1 is what ends
/// the program's wait for it.
fn classify(body: &[u8], final_byte: u8) -> Option<Query> {
    match (body, final_byte) {
        (b"" | b"0", b'c') => Some(Query::PrimaryAttributes),
        (b"5", b'n') => Some(Query::Status),
        (b"6", b'n') => Some(Query::CursorPosition),
        (_, b'p') => {
            let mode = body.strip_prefix(b"?")?.strip_suffix(b"$")?;
            Some(Query::Mode(std::str::from_utf8(mode).ok()?.parse().ok()?))
        }
        _ => None,
    }
}

/// The terminal's reply path for one pane: it sees everything the pane's child writes, and
/// says what the child is owed back.
///
/// It is a byte-at-a-time recogniser rather than a search over each chunk, because chunks are
/// whatever the reader thread happened to read: a query split over two of them is rare,
/// unreproducible, and would otherwise go unanswered.
#[derive(Default)]
pub struct Queries {
    /// The start of a sequence that has not reached its final byte yet, empty between them.
    pending: Vec<u8>,
}

impl Queries {
    /// Feeds one chunk of a pane's output to that pane's terminal and returns the bytes the
    /// terminal owes the child. Feeding is interleaved with recognition so that a cursor
    /// report describes where the cursor was when it was asked for, not where the rest of
    /// the chunk left it.
    pub fn feed(&mut self, chunk: &[u8], state: &mut TermState) -> Vec<u8> {
        let mut out = Vec::new();
        let mut fed = 0;
        for (i, byte) in chunk.iter().enumerate() {
            if let Some(query) = self.push(*byte) {
                state.feed(&chunk[fed..=i]);
                fed = i + 1;
                query.reply(state.cursor(), &mut out);
            }
        }
        state.feed(&chunk[fed..]);
        out
    }

    fn push(&mut self, byte: u8) -> Option<Query> {
        match self.pending.len() {
            0 => {
                if byte == ESC {
                    self.pending.push(byte);
                }
                None
            }
            1 => {
                if byte == b'[' {
                    self.pending.push(byte);
                } else {
                    self.pending.clear();
                    if byte == ESC {
                        self.pending.push(ESC);
                    }
                }
                None
            }
            _ => match byte {
                ESC => {
                    self.pending.clear();
                    self.pending.push(ESC);
                    None
                }
                0x20..=0x3f => {
                    self.pending.push(byte);
                    if self.pending.len() > MAX_PENDING {
                        self.pending.clear();
                    }
                    None
                }
                0x40..=0x7e => {
                    let query = classify(&self.pending[2..], byte);
                    self.pending.clear();
                    query
                }
                _ => {
                    self.pending.clear();
                    None
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use neotmux_proto::Size;

    use super::*;

    /// A terminal and its reply path, as a pane has them.
    fn pane() -> (Queries, TermState) {
        (
            Queries::default(),
            TermState::new(Size { rows: 4, cols: 20 }, 0),
        )
    }

    fn answer(input: &[u8]) -> Vec<u8> {
        let (mut queries, mut state) = pane();
        queries.feed(input, &mut state)
    }

    fn answer_text(input: &[u8]) -> String {
        String::from_utf8(answer(input)).expect("replies are ASCII")
    }

    #[test]
    fn primary_device_attributes_says_vt100_with_advanced_video() {
        assert_eq!(answer(b"\x1b[c"), b"\x1b[?1;2c");
    }

    /// `CSI 0 c` is the same query spelled with its default parameter.
    #[test]
    fn primary_device_attributes_with_an_explicit_default_parameter_is_the_same_query() {
        assert_eq!(answer(b"\x1b[0c"), b"\x1b[?1;2c");
    }

    #[test]
    fn a_status_report_says_the_terminal_is_well() {
        assert_eq!(answer(b"\x1b[5n"), b"\x1b[0n");
    }

    /// The one answer that has to come from real state: the pane's own cursor, one-based.
    #[test]
    fn a_cursor_report_gives_the_panes_own_cursor() {
        let (mut queries, mut state) = pane();
        assert_eq!(queries.feed(b"abc\x1b[6n", &mut state), b"\x1b[1;4R");
    }

    /// The cursor is read where the query was, not where the rest of the chunk left it.
    #[test]
    fn a_cursor_report_is_taken_at_the_query_not_at_the_end_of_the_chunk() {
        let (mut queries, mut state) = pane();
        assert_eq!(
            queries.feed(b"ab\x1b[6ncdefgh", &mut state),
            b"\x1b[1;3R",
            "the cursor moved on before the reply was composed"
        );
    }

    /// Every mode nvim asks about is one neotmux does not implement, and 0 is what says so.
    #[test]
    fn every_mode_query_says_the_mode_is_not_recognised() {
        for mode in [69u16, 2026, 2027, 2031, 2048] {
            let query = format!("\x1b[?{mode}$p");
            assert_eq!(
                answer_text(query.as_bytes()),
                format!("\x1b[?{mode};0$y"),
                "mode {mode}"
            );
        }
    }

    /// Answering this would claim an encoding neotmux never sends; the silence is the point.
    #[test]
    fn the_kitty_keyboard_query_is_left_unanswered() {
        assert!(answer(b"\x1b[?u").is_empty());
    }

    /// The whole run nvim sends, in one chunk: everything it waits on is either answered or
    /// deliberately not, and DA1 comes last so it still ends the wait.
    #[test]
    fn the_run_of_queries_nvim_sends_is_answered_in_order() {
        assert_eq!(
            answer_text(b"\x1b[5n\x1b[?u\x1b[?69$p\x1b[?2026$p\x1b[c"),
            "\x1b[0n\x1b[?69;0$y\x1b[?2026;0$y\x1b[?1;2c"
        );
    }

    /// Chunks are whatever the reader thread read, so a query can arrive one byte at a time.
    #[test]
    fn a_query_split_across_chunks_is_still_answered() {
        let (mut queries, mut state) = pane();
        let query = b"\x1b[?2026$p";
        let mut replies = Vec::new();
        for byte in query {
            replies.extend_from_slice(&queries.feed(&[*byte], &mut state));
        }
        assert_eq!(replies, b"\x1b[?2026;0$y");
    }

    #[test]
    fn every_query_survives_being_split_at_every_offset() {
        for query in [&b"\x1b[c"[..], b"\x1b[5n", b"\x1b[6n", b"\x1b[?2048$p"] {
            let whole = answer(query);
            for split in 1..query.len() {
                let (mut queries, mut state) = pane();
                let mut replies = queries.feed(&query[..split], &mut state);
                replies.extend_from_slice(&queries.feed(&query[split..], &mut state));
                assert_eq!(replies, whole, "{query:?} split at {split}");
            }
        }
    }

    /// A reply goes to the child's PTY, and a child with echo on — or a `cat` — sends it
    /// straight back. If any reply parsed as a query the two would trade bytes for ever.
    #[test]
    fn no_reply_is_itself_a_query() {
        let replies: [&[u8]; 4] = [b"\x1b[?1;2c", b"\x1b[0n", b"\x1b[3;7R", b"\x1b[?2026;0$y"];
        for reply in replies {
            assert!(
                answer(reply).is_empty(),
                "{reply:?} came back as a query and would loop"
            );
        }
    }

    /// Queries only; a terminal that answered the sequences it merely renders would be
    /// writing noise into the child's input.
    #[test]
    fn ordinary_output_is_never_answered() {
        for output in [
            &b"hello\r\n"[..],
            b"\x1b[1;31mred\x1b[0m",
            b"\x1b[2J\x1b[H",
            b"\x1b[?1049h",
            b"\x1b[?25l",
            b"\x1bP+q544e\x1b\\",
            b"\x1b]0;a title\x07",
        ] {
            assert!(answer(output).is_empty(), "{output:?} was answered");
        }
    }

    /// An unterminated sequence must not grow without bound, and must not swallow the query
    /// that follows it.
    #[test]
    fn a_runaway_sequence_is_abandoned_and_the_next_query_still_lands() {
        let (mut queries, mut state) = pane();
        let runaway = [&b"\x1b["[..], &[b'1'; 200]].concat();
        assert!(queries.feed(&runaway, &mut state).is_empty());
        assert!(queries.pending.len() <= MAX_PENDING);
        assert_eq!(queries.feed(b"\x1b[c", &mut state), b"\x1b[?1;2c");
    }

    /// Recognising queries must not cost the terminal any of the output around them.
    #[test]
    fn the_output_a_query_is_embedded_in_still_reaches_the_terminal() {
        let (mut queries, mut state) = pane();
        queries.feed(b"ab\x1b[ccd", &mut state);
        let mut grid = neotmux_proto::GridSnapshot::default();
        state.snapshot_into(&mut grid);
        let row: String = (0..4)
            .map(|col| grid.cell(0, col).expect("in bounds").text.as_str())
            .collect();
        assert_eq!(row, "abcd");
    }
}

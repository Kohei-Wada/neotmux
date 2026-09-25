//! Terminal bytes into [`Key`]s.
//!
//! A batch handed to [`Decoder::feed`] is everything the terminal had to give at that
//! moment — `Terminal::read` drains until the file stops being readable — and the decoder
//! leans on that in the one place a terminal input parser has no good answer.
//!
//! **Escape.** `0x1b` is both the Escape key and the first byte of almost every other key's
//! sequence, and nothing in the byte stream distinguishes them. The rule here: an `ESC` with
//! nothing after it *in a batch that was read to exhaustion* is the Escape key, and an `ESC`
//! with more bytes behind it begins a sequence. The cost is paid in two places. First, a
//! sequence torn across two reads — the terminal writing `ESC` and the rest of `ESC [ A` in
//! separate `write(2)`s with neotmux scheduled in between — is read as Escape followed by
//! the remainder as literal keys; terminals write a key's bytes in one call, so this needs a
//! very slow link to happen at all. Second, the converse: pressing Escape and then a letter
//! fast enough that both land in one read is `M-<letter>`, not two keys. What is *not* paid
//! is latency: Escape is dispatched the moment it is read, with no timer to wait out, which
//! is what a timeout-based parser trades for the same two mistakes. This is the same rule
//! crossterm's parser applied, so the behaviour here is the behaviour neotmux already had.
//!
//! **Sequences with no meaning here.** A well-formed sequence whose final byte neotmux has
//! no key for — a mouse report, a focus notification, a device status reply — is consumed
//! and discarded, whole. It is never passed through to the focused pane: the pane's child
//! did not ask for it (neotmux enables no such mode), so forwarding it would type garbage.
//! A malformed sequence loses its `ESC` and the rest is decoded again from the next byte.

use neotmux_proto::{Key, KeyCode, Modifiers};

/// The longest unfinished sequence worth holding for the next read. A `CSI` with more
/// parameter bytes than this is not a key neotmux has, so keeping it would only corrupt the
/// keystroke that follows it.
const MAX_PENDING: usize = 16;

/// What one decode of the front of a buffer found.
enum Step {
    /// A key, and the bytes it took.
    Key(Key, usize),
    /// Bytes that mean nothing here, dropped.
    Skip(usize),
    /// The start of something that has not all arrived.
    Incomplete,
}

/// Holds the tail of a batch that stopped mid-sequence, so a key torn across two reads is
/// still one key.
#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
}

impl Decoder {
    /// Decodes one fully drained batch of terminal bytes.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.pending.extend_from_slice(bytes);
        let mut keys = Vec::new();
        let mut at = 0;
        while at < self.pending.len() {
            match step(&self.pending[at..]) {
                Step::Key(key, used) => {
                    keys.push(key);
                    at += used;
                }
                Step::Skip(used) => at += used,
                Step::Incomplete => break,
            }
        }
        self.pending.drain(..at);
        if self.pending.len() > MAX_PENDING {
            self.pending.clear();
        }
        keys
    }
}

fn step(bytes: &[u8]) -> Step {
    match bytes.first() {
        Some(0x1b) => escape(bytes),
        Some(_) => single(bytes),
        None => Step::Incomplete,
    }
}

/// One byte that is not the start of an escape sequence: a control character, or the first
/// byte of a character.
fn single(bytes: &[u8]) -> Step {
    let byte = bytes[0];
    let ctrl = |c: char| Step::Key(Key::new(KeyCode::Char(c), Modifiers::CTRL), 1);
    match byte {
        b'\r' => Step::Key(Key::plain(KeyCode::Enter), 1),
        b'\t' => Step::Key(Key::plain(KeyCode::Tab), 1),
        0x7f => Step::Key(Key::plain(KeyCode::Backspace), 1),
        0 => ctrl(' '),
        // `\n` is in this range and is reached as C-j: the line discipline is in raw mode,
        // so nothing turns a newline into Enter on the way here.
        0x01..=0x1a => ctrl((byte - 1 + b'a') as char),
        0x1c..=0x1f => ctrl((byte - 0x1c + b'4') as char),
        _ => utf8(bytes),
    }
}

fn utf8(bytes: &[u8]) -> Step {
    let Some(len) = utf8_len(bytes[0]) else {
        return Step::Skip(1);
    };
    if bytes.len() < len {
        return Step::Incomplete;
    }
    match std::str::from_utf8(&bytes[..len])
        .ok()
        .and_then(|s| s.chars().next())
    {
        Some(c) => Step::Key(Key::plain(KeyCode::Char(c)), len),
        None => Step::Skip(1),
    }
}

fn utf8_len(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn escape(bytes: &[u8]) -> Step {
    match bytes.get(1) {
        // Read to exhaustion with nothing behind it: the key, not the start of a sequence.
        None => Step::Key(Key::plain(KeyCode::Esc), 1),
        Some(b'[') => csi(bytes),
        Some(b'O') => ss3(bytes),
        // `ESC ESC` is Escape and then whatever the second one turns out to begin.
        Some(0x1b) => Step::Key(Key::plain(KeyCode::Esc), 1),
        Some(_) => match single(&bytes[1..]) {
            Step::Key(key, used) => Step::Key(key.with_alt(), used + 1),
            Step::Skip(used) => Step::Skip(used + 1),
            Step::Incomplete => Step::Incomplete,
        },
    }
}

/// `ESC [ <parameter bytes> <intermediate bytes> <final byte>`, as ECMA-48 defines it, so
/// that a sequence with no meaning here is still consumed whole rather than leaking its tail.
fn csi(bytes: &[u8]) -> Step {
    let mut end = 2;
    while end < bytes.len() && (0x30..=0x3f).contains(&bytes[end]) {
        end += 1;
    }
    while end < bytes.len() && (0x20..=0x2f).contains(&bytes[end]) {
        end += 1;
    }
    let Some(&final_byte) = bytes.get(end) else {
        return Step::Incomplete;
    };
    // Not a final byte at all: the sequence was cut short, so drop what has been read of it
    // and start again at the byte that broke it — which is often the next sequence's `ESC`.
    if !(0x40..=0x7e).contains(&final_byte) {
        return Step::Skip(end);
    }
    let used = end + 1;
    let Some(params) = params(&bytes[2..end]) else {
        return Step::Skip(used);
    };
    let param = |at: usize| params.get(at).copied().filter(|n| *n > 0).unwrap_or(1);
    let mods = Modifiers::from_param(param(1));

    let code = match final_byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        // Shift-Tab's own sequence carries no modifier parameter, and saying it is shifted
        // would make it a different key from the `BackTab` the rest of the program binds.
        b'Z' => return Step::Key(Key::plain(KeyCode::BackTab), used),
        // kitty / xterm `CSI u` keyboard encoding: parameter 0 is a Unicode codepoint, 1 a
        // kitty modifier mask. A bare Escape arrives as `CSI 27 u`, which the catch-all below
        // used to swallow outright — the key never reached the pane.
        b'u' => {
            let Some(&codepoint) = params.first() else {
                return Step::Skip(used);
            };
            let mask = params.get(1).copied().unwrap_or(0);
            let mods = Modifiers::from_param(mask + 1); // kitty mask +1 == xterm parameter
            let Some(code) = key_from_codepoint(codepoint) else {
                return Step::Skip(used);
            };
            return Step::Key(as_escape(Key::new(code, mods)), used);
        }
        b'~' => match param(0) {
            1 | 7 => KeyCode::Home,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            4 | 8 => KeyCode::End,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            15 => KeyCode::F(5),
            17 => KeyCode::F(6),
            18 => KeyCode::F(7),
            19 => KeyCode::F(8),
            20 => KeyCode::F(9),
            21 => KeyCode::F(10),
            23 => KeyCode::F(11),
            24 => KeyCode::F(12),
            _ => return Step::Skip(used),
        },
        _ => return Step::Skip(used),
    };
    Step::Key(Key::new(code, mods), used)
}

/// `C-[` is Escape, and has been since before there were arrow keys: a terminal sends `0x1b`
/// for both, which is why `Esc` and `C-[` are the same key to every program that has ever read
/// a tty. A terminal in the kitty keyboard protocol can afford to tell them apart and does —
/// `[` with the control modifier, or Escape carrying one — and the rest of neotmux would then
/// see a key nothing binds while the person is pressing the one thing that means "stop".
///
/// So they are folded back together here, at the edge, rather than every plugin being asked to
/// know about it. What the pane receives is unchanged: `Esc` is `0x1b`, which is what `C-[`
/// would have sent anyway.
fn as_escape(key: Key) -> Key {
    if key.mods.ctrl && matches!(key.code, KeyCode::Char('[') | KeyCode::Esc) {
        return Key::plain(KeyCode::Esc);
    }
    key
}

/// `CSI u` (kitty / xterm keyboard protocol) names a key by its Unicode codepoint: ordinary
/// characters are their own codepoint (`CSI 97 u` = `a`), named keys use the codepoint the
/// protocol assigns (Escape = 27, arrows = the arrow glyphs). A codepoint with no key yields
/// `None`, so the sequence is consumed and dropped rather than leaking bytes.
fn key_from_codepoint(cp: u16) -> Option<KeyCode> {
    match cp {
        0x09 => Some(KeyCode::Tab),
        0x0d => Some(KeyCode::Enter),
        0x1b => Some(KeyCode::Esc),
        0x08 => Some(KeyCode::Backspace),
        0x7f => Some(KeyCode::Delete),
        0x2190 => Some(KeyCode::Left),
        0x2191 => Some(KeyCode::Up),
        0x2192 => Some(KeyCode::Right),
        0x2193 => Some(KeyCode::Down),
        _ => char::from_u32(u32::from(cp))
            .filter(|c| !c.is_control())
            .map(KeyCode::Char),
    }
}

fn params(raw: &[u8]) -> Option<Vec<u16>> {
    if raw.iter().any(|b| !matches!(b, b'0'..=b'9' | b';')) {
        return None;
    }
    if raw.is_empty() {
        return Some(Vec::new());
    }
    Some(
        raw.split(|b| *b == b';')
            .map(|part| {
                std::str::from_utf8(part)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0)
            })
            .collect(),
    )
}

/// `ESC O <final byte>`: the application-cursor-mode arrows and the unmodified F1–F4.
fn ss3(bytes: &[u8]) -> Step {
    let Some(&final_byte) = bytes.get(2) else {
        return Step::Incomplete;
    };
    let code = match final_byte {
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        _ => return Step::Skip(3),
    };
    Step::Key(Key::plain(code), 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> Vec<Key> {
        Decoder::default().feed(bytes)
    }

    fn one(bytes: &[u8]) -> Key {
        let keys = decode(bytes);
        assert_eq!(
            keys.len(),
            1,
            "expected one key from {bytes:?}, got {keys:?}"
        );
        keys[0]
    }

    #[test]
    fn printable_characters_decode_one_per_character() {
        assert_eq!(one(b"a"), Key::plain(KeyCode::Char('a')));
        assert_eq!(
            decode(b"abc"),
            vec![
                Key::plain(KeyCode::Char('a')),
                Key::plain(KeyCode::Char('b')),
                Key::plain(KeyCode::Char('c')),
            ]
        );
    }

    #[test]
    fn a_multibyte_character_is_one_key() {
        assert_eq!(one("あ".as_bytes()), Key::plain(KeyCode::Char('あ')));
        assert_eq!(one("🦀".as_bytes()), Key::plain(KeyCode::Char('🦀')));
    }

    /// A read can stop between the bytes of one character when the writer is still writing,
    /// which a paste big enough to outrun the drain does.
    #[test]
    fn a_character_torn_across_two_reads_is_still_one_key() {
        let bytes = "あ".as_bytes();
        let mut decoder = Decoder::default();
        assert!(decoder.feed(&bytes[..1]).is_empty());
        assert_eq!(
            decoder.feed(&bytes[1..]),
            vec![Key::plain(KeyCode::Char('あ'))]
        );
    }

    #[test]
    fn control_characters_decode_to_their_letter() {
        assert_eq!(one(&[0x02]), Key::new(KeyCode::Char('b'), Modifiers::CTRL));
        assert_eq!(one(&[0x03]), Key::new(KeyCode::Char('c'), Modifiers::CTRL));
        assert_eq!(one(&[0x00]), Key::new(KeyCode::Char(' '), Modifiers::CTRL));
        assert_eq!(one(&[0x1c]), Key::new(KeyCode::Char('4'), Modifiers::CTRL));
    }

    #[test]
    fn the_named_control_characters_are_their_own_keys() {
        assert_eq!(one(b"\r"), Key::plain(KeyCode::Enter));
        assert_eq!(one(b"\t"), Key::plain(KeyCode::Tab));
        assert_eq!(one(&[0x7f]), Key::plain(KeyCode::Backspace));
    }

    /// Raw mode leaves `\n` alone, so it is the control character and not Enter.
    #[test]
    fn a_newline_is_control_j_not_enter() {
        assert_eq!(one(b"\n"), Key::new(KeyCode::Char('j'), Modifiers::CTRL));
    }

    #[test]
    fn a_lone_escape_at_the_end_of_a_batch_is_the_escape_key() {
        assert_eq!(one(&[0x1b]), Key::plain(KeyCode::Esc));
        assert_eq!(
            decode(b"a\x1b"),
            vec![Key::plain(KeyCode::Char('a')), Key::plain(KeyCode::Esc)]
        );
    }

    #[test]
    fn an_escape_that_begins_a_sequence_is_not_the_escape_key() {
        assert_eq!(one(b"\x1b[A"), Key::plain(KeyCode::Up));
        assert_eq!(one(b"\x1bOP"), Key::plain(KeyCode::F(1)));
        assert_eq!(one(b"\x1b[3~"), Key::plain(KeyCode::Delete));
    }

    /// `CSI u` (the kitty / xterm keyboard protocol) names a key by its Unicode codepoint
    // (decimal in the parameter bytes, like every other `CSI` parameter). A bare `27` is
    // Escape; the reported "ctl-[ never arrives" bug was this whole sequence being dropped.
    #[test]
    fn a_csi_u_escape_decodes_to_the_escape_key() {
        assert_eq!(one(b"\x1b[27u"), Key::plain(KeyCode::Esc));
    }

    #[test]
    fn a_csi_u_character_decodes_to_that_character() {
        assert_eq!(one(b"\x1b[97u"), Key::plain(KeyCode::Char('a')));
        // `あ` is U+3042 = 12354.
        assert_eq!(one(b"\x1b[12354u"), Key::plain(KeyCode::Char('あ')));
    }

    /// The second parameter is the kitty modifier mask; alt is bit 2. A plain character has
    /// no modifier parameter at all.
    #[test]
    fn a_csi_u_modification_is_read_back() {
        assert_eq!(
            one(b"\x1b[97;2u"),
            Key::new(KeyCode::Char('a'), Modifiers::ALT)
        );
        // Escape carrying control is Escape — see `as_escape`, and the test below it.
        assert_eq!(one(b"\x1b[27;4u"), Key::plain(KeyCode::Esc));
    }

    /// A named key by its protocol codepoint: Up = U+2191 = 8593.
    #[test]
    fn a_csi_u_arrow_decodes_to_the_named_key() {
        assert_eq!(one(b"\x1b[8593u"), Key::plain(KeyCode::Up));
    }

    /// A codepoint with no key (a control we don't name) is still consumed whole, so its
    /// tail never leaks into the next keystroke.
    #[test]
    fn a_csi_u_with_no_key_is_dropped_whole() {
        // codepoint 5 (ENQ) maps to nothing
        assert_eq!(decode(b"\x1b[5u"), vec![]);
        // a key really following it survives
        assert_eq!(decode(b"\x1b[5ua"), vec![Key::plain(KeyCode::Char('a'))]);
    }

    #[test]
    fn escape_then_a_character_in_one_batch_is_alt() {
        assert_eq!(one(b"\x1ba"), Key::new(KeyCode::Char('a'), Modifiers::ALT));
        assert_eq!(
            one(&[0x1b, 0x02]),
            Key::new(
                KeyCode::Char('b'),
                Modifiers {
                    ctrl: true,
                    alt: true,
                    shift: false
                }
            )
        );
    }

    #[test]
    fn a_sequence_torn_across_two_reads_is_still_one_key() {
        let mut decoder = Decoder::default();
        assert!(decoder.feed(b"\x1b[").is_empty());
        assert_eq!(
            decoder.feed(b"1;5D"),
            vec![Key::new(KeyCode::Left, Modifiers::CTRL)]
        );
    }

    #[test]
    fn the_arrows_decode_from_both_csi_and_ss3() {
        for (csi, ss3, code) in [
            (&b"\x1b[A"[..], &b"\x1bOA"[..], KeyCode::Up),
            (b"\x1b[B", b"\x1bOB", KeyCode::Down),
            (b"\x1b[C", b"\x1bOC", KeyCode::Right),
            (b"\x1b[D", b"\x1bOD", KeyCode::Left),
            (b"\x1b[H", b"\x1bOH", KeyCode::Home),
            (b"\x1b[F", b"\x1bOF", KeyCode::End),
        ] {
            assert_eq!(one(csi), Key::plain(code));
            assert_eq!(one(ss3), Key::plain(code));
        }
    }

    #[test]
    fn the_tilde_keys_decode_to_their_names() {
        for (bytes, code) in [
            (&b"\x1b[2~"[..], KeyCode::Insert),
            (b"\x1b[3~", KeyCode::Delete),
            (b"\x1b[5~", KeyCode::PageUp),
            (b"\x1b[6~", KeyCode::PageDown),
            (b"\x1b[1~", KeyCode::Home),
            (b"\x1b[4~", KeyCode::End),
            (b"\x1b[15~", KeyCode::F(5)),
            (b"\x1b[17~", KeyCode::F(6)),
            (b"\x1b[18~", KeyCode::F(7)),
            (b"\x1b[19~", KeyCode::F(8)),
            (b"\x1b[20~", KeyCode::F(9)),
            (b"\x1b[21~", KeyCode::F(10)),
            (b"\x1b[23~", KeyCode::F(11)),
            (b"\x1b[24~", KeyCode::F(12)),
        ] {
            assert_eq!(one(bytes), Key::plain(code));
        }
    }

    #[test]
    fn the_low_function_keys_decode_from_both_forms() {
        for (ss3, csi, n) in [
            (&b"\x1bOP"[..], &b"\x1b[1;5P"[..], 1u8),
            (b"\x1bOQ", b"\x1b[1;5Q", 2),
            (b"\x1bOR", b"\x1b[1;5R", 3),
            (b"\x1bOS", b"\x1b[1;5S", 4),
        ] {
            assert_eq!(one(ss3), Key::plain(KeyCode::F(n)));
            assert_eq!(one(csi), Key::new(KeyCode::F(n), Modifiers::CTRL));
        }
    }

    #[test]
    fn shift_tab_decodes_to_back_tab() {
        assert_eq!(one(b"\x1b[Z"), Key::plain(KeyCode::BackTab));
    }

    #[test]
    fn the_modifier_parameter_is_read_back() {
        assert_eq!(one(b"\x1b[1;5D"), Key::new(KeyCode::Left, Modifiers::CTRL));
        assert_eq!(one(b"\x1b[1;3A"), Key::new(KeyCode::Up, Modifiers::ALT));
        assert_eq!(one(b"\x1b[1;2A"), Key::new(KeyCode::Up, Modifiers::SHIFT));
        assert_eq!(
            one(b"\x1b[1;8A"),
            Key::new(
                KeyCode::Up,
                Modifiers {
                    shift: true,
                    alt: true,
                    ctrl: true
                }
            )
        );
        assert_eq!(
            one(b"\x1b[5;5~"),
            Key::new(KeyCode::PageUp, Modifiers::CTRL)
        );
        assert_eq!(one(b"\x1b[24;3~"), Key::new(KeyCode::F(12), Modifiers::ALT));
        assert_eq!(one(b"\x1b[1;5H"), Key::new(KeyCode::Home, Modifiers::CTRL));
    }

    /// Neither mouse reporting nor bracketed paste is ever switched on, so these arrive only
    /// from a terminal doing it unasked — and the pane's child did not ask either.
    #[test]
    fn a_sequence_with_no_key_here_is_swallowed_whole() {
        assert!(decode(b"\x1b[<0;10;5M").is_empty());
        assert!(decode(b"\x1b[200~").is_empty());
        assert!(decode(b"\x1b[I").is_empty());
        assert_eq!(
            decode(b"\x1b[<0;10;5Ma"),
            vec![Key::plain(KeyCode::Char('a'))],
            "the tail of an unparsed sequence leaked into the keys"
        );
    }

    #[test]
    fn a_malformed_sequence_costs_only_its_escape() {
        assert_eq!(
            decode(b"\x1b[\x1b[A"),
            vec![Key::plain(KeyCode::Up)],
            "a truncated sequence swallowed the one that followed it"
        );
    }

    /// A prefix that never completes must not corrupt every key after it.
    #[test]
    fn an_unfinished_sequence_is_not_held_for_ever() {
        let mut decoder = Decoder::default();
        assert!(decoder.feed(b"\x1b[123456789012345678").is_empty());
        assert_eq!(decoder.feed(b"a"), vec![Key::plain(KeyCode::Char('a'))]);
    }

    #[test]
    fn a_paste_of_many_characters_decodes_to_that_many_keys() {
        let paste = "x".repeat(4096);
        let keys = decode(paste.as_bytes());
        assert_eq!(keys.len(), 4096);
        assert!(keys.iter().all(|k| *k == Key::plain(KeyCode::Char('x'))));
    }
    /// `C-[` is Escape wherever it comes from. A terminal that sends `0x1b` says so by saying
    /// `0x1b`; one in the kitty protocol says `[` with control, or Escape with control, and
    /// both mean the key the person pressed to stop whatever is happening.
    #[test]
    fn control_bracket_is_escape_however_the_terminal_spells_it() {
        assert_eq!(one(&[0x1b]), Key::plain(KeyCode::Esc));
        // CSI 91 ; 5 u — `[` with the control modifier
        assert_eq!(one(b"\x1b[91;5u"), Key::plain(KeyCode::Esc));
        // CSI 27 ; 5 u — Escape itself, carrying one
        assert_eq!(one(b"\x1b[27;5u"), Key::plain(KeyCode::Esc));
        // and an ordinary `[` is still an ordinary `[`
        assert_eq!(one(b"\x1b[91u"), Key::plain(KeyCode::Char('[')));
    }
}

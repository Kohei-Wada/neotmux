//! Keys out: the spec a binding is looked up by, and the bytes a pane's child is typed.

use neotmux_proto::keys::{Key, KeyCode};

/// `ESC [ A` normally, `ESC O A` when the pane has asked for the application form — and the
/// CSI form either way once a modifier is on it, because `ESC O` has nowhere to put the
/// parameter. That is what xterm does, and what every application that reads these expects.
fn cursor_key(final_byte: u8, modifier: u8, application: bool) -> Vec<u8> {
    if modifier > 1 {
        format!("\x1b[1;{modifier}{}", final_byte as char).into_bytes()
    } else if application {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde_key(code: u8, modifier: u8) -> Vec<u8> {
    if modifier > 1 {
        format!("\x1b[{code};{modifier}~").into_bytes()
    } else {
        format!("\x1b[{code}~").into_bytes()
    }
}

fn low_function_key(letter: u8, modifier: u8) -> Vec<u8> {
    if modifier > 1 {
        format!("\x1b[1;{modifier}{}", letter as char).into_bytes()
    } else {
        vec![0x1b, b'O', letter]
    }
}

/// Turns a key into the spec string `LuaSubsystem::dispatch_key` expects
/// (e.g. `C-x`, `M-Left`, `.`), matching what `keymap::normalise` produces.
pub fn key_spec(key: &Key) -> String {
    let key_part = match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::F(n) => format!("F{n}"),
    };

    let mut spec = String::new();
    if key.mods.ctrl {
        spec.push_str("C-");
    }
    if key.mods.alt {
        spec.push_str("M-");
    }
    // A shifted character is already the shifted character, so saying so again would make
    // `A` and `S-A` two different bindings for one keystroke.
    if key.mods.shift && !matches!(key.code, KeyCode::Char(_)) {
        spec.push_str("S-");
    }
    spec.push_str(&key_part);
    spec
}

/// The bytes a pane is sent for a key.
///
/// `application_cursor` is the pane's own DECCKM: for the pane, neotmux is the terminal, so
/// the pane's mode decides whether a cursor key is written `ESC [ A` or `ESC O A`. Normalising
/// the two forms on the way *in* is right and stays — that is the outer terminal's mode, and
/// neotmux is the application there — and this is the other end of the same rule.
pub fn encode_key(key: Key, application_cursor: bool) -> Option<Vec<u8>> {
    let modifier = key.mods.param();

    let mut bytes = match key.code {
        KeyCode::Char(c) if key.mods.ctrl => {
            let upper = c.to_ascii_uppercase();
            if upper.is_ascii() {
                vec![(upper as u8) & 0x1f]
            } else {
                return None;
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => cursor_key(b'A', modifier, application_cursor),
        KeyCode::Down => cursor_key(b'B', modifier, application_cursor),
        KeyCode::Right => cursor_key(b'C', modifier, application_cursor),
        KeyCode::Left => cursor_key(b'D', modifier, application_cursor),
        KeyCode::Home => cursor_key(b'H', modifier, application_cursor),
        KeyCode::End => cursor_key(b'F', modifier, application_cursor),
        KeyCode::PageUp => tilde_key(5, modifier),
        KeyCode::PageDown => tilde_key(6, modifier),
        KeyCode::Insert => tilde_key(2, modifier),
        KeyCode::Delete => tilde_key(3, modifier),
        KeyCode::F(n @ 1..=4) => low_function_key(b'P' + (n - 1), modifier),
        KeyCode::F(n @ 5..=12) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                _ => 24,
            };
            tilde_key(code, modifier)
        }
        KeyCode::F(_) => return None,
    };

    let has_no_parameterised_form = matches!(
        key.code,
        KeyCode::Char(_)
            | KeyCode::Enter
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Backspace
            | KeyCode::Esc
    );
    if key.mods.alt && has_no_parameterised_form {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use neotmux_proto::keys::{Key, KeyCode, Modifiers};

    use super::encode_key;

    fn key(code: KeyCode) -> Key {
        Key::plain(code)
    }

    #[test]
    fn plain_characters_are_utf8() {
        assert_eq!(encode_key(key(KeyCode::Char('a')), false).unwrap(), b"a");
        assert_eq!(
            encode_key(key(KeyCode::Char('あ')), false).unwrap(),
            "あ".as_bytes()
        );
    }

    #[test]
    fn control_characters_are_masked() {
        let ctrl_c = Key::new(KeyCode::Char('c'), Modifiers::CTRL);
        assert_eq!(encode_key(ctrl_c, false).unwrap(), vec![0x03]);
        let ctrl_d = Key::new(KeyCode::Char('d'), Modifiers::CTRL);
        assert_eq!(encode_key(ctrl_d, false).unwrap(), vec![0x04]);
    }

    #[test]
    fn named_keys_map_to_their_bytes() {
        assert_eq!(encode_key(key(KeyCode::Enter), false).unwrap(), b"\r");
        assert_eq!(encode_key(key(KeyCode::Tab), false).unwrap(), b"\t");
        assert_eq!(
            encode_key(key(KeyCode::Backspace), false).unwrap(),
            vec![0x7f]
        );
        assert_eq!(encode_key(key(KeyCode::Esc), false).unwrap(), vec![0x1b]);
    }

    #[test]
    fn arrows_use_normal_cursor_mode() {
        assert_eq!(encode_key(key(KeyCode::Up), false).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(key(KeyCode::Down), false).unwrap(), b"\x1b[B");
        assert_eq!(encode_key(key(KeyCode::Right), false).unwrap(), b"\x1b[C");
        assert_eq!(encode_key(key(KeyCode::Left), false).unwrap(), b"\x1b[D");
    }

    #[test]
    fn alt_prefixes_with_escape() {
        let alt_a = Key::new(KeyCode::Char('a'), Modifiers::ALT);
        assert_eq!(encode_key(alt_a, false).unwrap(), b"\x1ba");
    }

    #[test]
    fn alt_prefixes_keys_with_no_parameterised_csi_form() {
        assert_eq!(
            encode_key(Key::new(KeyCode::Backspace, Modifiers::ALT), false).unwrap(),
            vec![0x1b, 0x7f]
        );
        assert_eq!(
            encode_key(Key::new(KeyCode::Enter, Modifiers::ALT), false).unwrap(),
            b"\x1b\r"
        );
        assert_eq!(
            encode_key(Key::new(KeyCode::Esc, Modifiers::ALT), false).unwrap(),
            vec![0x1b, 0x1b]
        );
    }

    #[test]
    fn alt_on_keys_with_a_parameterised_csi_form_uses_the_modifier_not_an_escape_prefix() {
        assert_eq!(
            encode_key(Key::new(KeyCode::Left, Modifiers::ALT), false).unwrap(),
            b"\x1b[1;3D"
        );
    }

    #[test]
    fn unsupported_keys_return_none() {
        assert!(encode_key(key(KeyCode::F(13)), false).is_none());
    }

    fn modified(code: KeyCode, mods: Modifiers) -> Key {
        Key::new(code, mods)
    }

    #[test]
    fn unmodified_named_keys_keep_their_encodings() {
        assert_eq!(encode_key(key(KeyCode::BackTab), false).unwrap(), b"\x1b[Z");
        assert_eq!(encode_key(key(KeyCode::Home), false).unwrap(), b"\x1b[H");
        assert_eq!(encode_key(key(KeyCode::End), false).unwrap(), b"\x1b[F");
        assert_eq!(encode_key(key(KeyCode::PageUp), false).unwrap(), b"\x1b[5~");
        assert_eq!(
            encode_key(key(KeyCode::PageDown), false).unwrap(),
            b"\x1b[6~"
        );
        assert_eq!(encode_key(key(KeyCode::Insert), false).unwrap(), b"\x1b[2~");
        assert_eq!(encode_key(key(KeyCode::Delete), false).unwrap(), b"\x1b[3~");
        assert_eq!(encode_key(key(KeyCode::F(1)), false).unwrap(), b"\x1bOP");
        assert_eq!(encode_key(key(KeyCode::F(2)), false).unwrap(), b"\x1bOQ");
        assert_eq!(encode_key(key(KeyCode::F(3)), false).unwrap(), b"\x1bOR");
        assert_eq!(encode_key(key(KeyCode::F(4)), false).unwrap(), b"\x1bOS");
        assert_eq!(encode_key(key(KeyCode::F(5)), false).unwrap(), b"\x1b[15~");
        assert_eq!(encode_key(key(KeyCode::F(6)), false).unwrap(), b"\x1b[17~");
        assert_eq!(encode_key(key(KeyCode::F(7)), false).unwrap(), b"\x1b[18~");
        assert_eq!(encode_key(key(KeyCode::F(8)), false).unwrap(), b"\x1b[19~");
        assert_eq!(encode_key(key(KeyCode::F(9)), false).unwrap(), b"\x1b[20~");
        assert_eq!(encode_key(key(KeyCode::F(10)), false).unwrap(), b"\x1b[21~");
        assert_eq!(encode_key(key(KeyCode::F(11)), false).unwrap(), b"\x1b[23~");
        assert_eq!(encode_key(key(KeyCode::F(12)), false).unwrap(), b"\x1b[24~");
    }

    #[test]
    fn ctrl_arrows_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Left, Modifiers::CTRL), false).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Right, Modifiers::CTRL), false).unwrap(),
            b"\x1b[1;5C"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Up, Modifiers::CTRL), false).unwrap(),
            b"\x1b[1;5A"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Down, Modifiers::CTRL), false).unwrap(),
            b"\x1b[1;5B"
        );
    }

    #[test]
    fn alt_arrows_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Up, Modifiers::ALT), false).unwrap(),
            b"\x1b[1;3A"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Left, Modifiers::ALT), false).unwrap(),
            b"\x1b[1;3D"
        );
    }

    #[test]
    fn shift_arrows_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Up, Modifiers::SHIFT), false).unwrap(),
            b"\x1b[1;2A"
        );
    }

    #[test]
    fn combined_modifiers_sum_into_the_parameter() {
        let ctrl_alt_shift = Modifiers {
            shift: true,
            alt: true,
            ctrl: true,
        };
        assert_eq!(
            encode_key(modified(KeyCode::Up, ctrl_alt_shift), false).unwrap(),
            b"\x1b[1;8A"
        );
    }

    #[test]
    fn modified_home_and_end_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Home, Modifiers::CTRL), false).unwrap(),
            b"\x1b[1;5H"
        );
        assert_eq!(
            encode_key(modified(KeyCode::End, Modifiers::CTRL), false).unwrap(),
            b"\x1b[1;5F"
        );
    }

    #[test]
    fn modified_tilde_keys_add_the_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::PageUp, Modifiers::CTRL), false).unwrap(),
            b"\x1b[5;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::PageDown, Modifiers::CTRL), false).unwrap(),
            b"\x1b[6;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Insert, Modifiers::CTRL), false).unwrap(),
            b"\x1b[2;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Delete, Modifiers::CTRL), false).unwrap(),
            b"\x1b[3;5~"
        );
    }

    #[test]
    fn modified_low_function_keys_use_the_csi_form() {
        assert_eq!(
            encode_key(modified(KeyCode::F(1), Modifiers::CTRL), false).unwrap(),
            b"\x1b[1;5P"
        );
        assert_eq!(
            encode_key(modified(KeyCode::F(4), Modifiers::ALT), false).unwrap(),
            b"\x1b[1;3S"
        );
    }

    #[test]
    fn modified_high_function_keys_add_the_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::F(5), Modifiers::CTRL), false).unwrap(),
            b"\x1b[15;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::F(12), Modifiers::ALT), false).unwrap(),
            b"\x1b[24;3~"
        );
    }

    #[test]
    fn key_spec_builds_control_and_meta_specs() {
        use super::key_spec;

        let ctrl_x = Key::new(KeyCode::Char('x'), Modifiers::CTRL);
        assert_eq!(key_spec(&ctrl_x), "C-x");

        let alt_left = modified(KeyCode::Left, Modifiers::ALT);
        assert_eq!(key_spec(&alt_left), "M-Left");

        let dot = key(KeyCode::Char('.'));
        assert_eq!(key_spec(&dot), ".");
    }

    /// Every key form the decoder produces has to survive the trip back to the bytes a
    /// terminal would have sent, because that is the trip a keystroke makes into a pane.
    #[test]
    fn decoding_bytes_and_encoding_them_again_is_the_identity() {
        for bytes in [
            &b"a"[..],
            "あ".as_bytes(),
            &[0x02],
            b"\r",
            b"\t",
            &[0x7f],
            &[0x1b],
            b"\x1ba",
            b"\x1b[A",
            b"\x1b[B",
            b"\x1b[C",
            b"\x1b[D",
            b"\x1b[H",
            b"\x1b[F",
            b"\x1b[Z",
            b"\x1b[2~",
            b"\x1b[3~",
            b"\x1b[5~",
            b"\x1b[6~",
            b"\x1b[15~",
            b"\x1b[24~",
            b"\x1bOP",
            b"\x1bOS",
            b"\x1b[1;5D",
            b"\x1b[1;3A",
            b"\x1b[1;2A",
            b"\x1b[1;8A",
            b"\x1b[5;5~",
            b"\x1b[24;3~",
            b"\x1b[1;5P",
        ] {
            let keys = crate::client::decode::Decoder::default().feed(bytes);
            assert_eq!(keys.len(), 1, "{bytes:?} did not decode to one key");
            assert_eq!(
                encode_key(keys[0], false).as_deref(),
                Some(bytes),
                "{bytes:?} did not survive the round trip as {:?}",
                keys[0]
            );
        }
    }

    /// For the pane, neotmux is the terminal, so the pane's own DECCKM decides the form. vim,
    /// less and readline's application mode all set it, and all four cursor keys — Home and
    /// End with them — move to `ESC O x` when they do.
    #[test]
    fn a_pane_in_application_cursor_mode_is_sent_the_ss3_form() {
        for (code, final_byte) in [
            (KeyCode::Up, b'A'),
            (KeyCode::Down, b'B'),
            (KeyCode::Right, b'C'),
            (KeyCode::Left, b'D'),
            (KeyCode::Home, b'H'),
            (KeyCode::End, b'F'),
        ] {
            assert_eq!(
                encode_key(key(code), false).unwrap(),
                vec![0x1b, b'[', final_byte],
                "{code:?} out of application mode"
            );
            assert_eq!(
                encode_key(key(code), true).unwrap(),
                vec![0x1b, b'O', final_byte],
                "{code:?} in application mode"
            );
        }
    }

    /// `ESC O` has nowhere to put a parameter, so a modified cursor key is the CSI form in
    /// both modes. That is what xterm does and what everything reading these expects.
    #[test]
    fn a_modifier_keeps_the_csi_form_in_either_mode() {
        for application in [false, true] {
            assert_eq!(
                encode_key(modified(KeyCode::Left, Modifiers::CTRL), application).unwrap(),
                b"\x1b[1;5D",
                "application={application}"
            );
        }
    }
}

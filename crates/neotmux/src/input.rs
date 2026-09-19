use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn modifier_param(modifiers: KeyModifiers) -> u8 {
    let mut param = 1;
    if modifiers.contains(KeyModifiers::SHIFT) {
        param += 1;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        param += 2;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        param += 4;
    }
    param
}

fn cursor_key(final_byte: u8, modifier: u8) -> Vec<u8> {
    if modifier > 1 {
        format!("\x1b[1;{modifier}{}", final_byte as char).into_bytes()
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

/// Turns a key event into the spec string `LuaSubsystem::dispatch_key` expects
/// (e.g. `C-x`, `M-Left`, `.`), matching what `keymap::normalise` produces.
pub fn key_spec(key: &KeyEvent) -> Option<String> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }

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
        _ => return None,
    };

    let mut spec = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        spec.push_str("C-");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        spec.push_str("M-");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) && !matches!(key.code, KeyCode::Char(_)) {
        spec.push_str("S-");
    }
    spec.push_str(&key_part);
    Some(spec)
}

pub fn encode_key(key: KeyEvent) -> Option<Vec<u8>> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }

    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let modifier = modifier_param(key.modifiers);

    let mut bytes = match key.code {
        KeyCode::Char(c) if ctrl => {
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
        KeyCode::Up => cursor_key(b'A', modifier),
        KeyCode::Down => cursor_key(b'B', modifier),
        KeyCode::Right => cursor_key(b'C', modifier),
        KeyCode::Left => cursor_key(b'D', modifier),
        KeyCode::Home => cursor_key(b'H', modifier),
        KeyCode::End => cursor_key(b'F', modifier),
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
        _ => return None,
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
    if alt && has_no_parameterised_form {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::encode_key;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn plain_characters_are_utf8() {
        assert_eq!(encode_key(key(KeyCode::Char('a'))).unwrap(), b"a");
        assert_eq!(
            encode_key(key(KeyCode::Char('あ'))).unwrap(),
            "あ".as_bytes()
        );
    }

    #[test]
    fn control_characters_are_masked() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(ctrl_c).unwrap(), vec![0x03]);
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(ctrl_d).unwrap(), vec![0x04]);
    }

    #[test]
    fn named_keys_map_to_their_bytes() {
        assert_eq!(encode_key(key(KeyCode::Enter)).unwrap(), b"\r");
        assert_eq!(encode_key(key(KeyCode::Tab)).unwrap(), b"\t");
        assert_eq!(encode_key(key(KeyCode::Backspace)).unwrap(), vec![0x7f]);
        assert_eq!(encode_key(key(KeyCode::Esc)).unwrap(), vec![0x1b]);
    }

    #[test]
    fn arrows_use_normal_cursor_mode() {
        assert_eq!(encode_key(key(KeyCode::Up)).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(key(KeyCode::Down)).unwrap(), b"\x1b[B");
        assert_eq!(encode_key(key(KeyCode::Right)).unwrap(), b"\x1b[C");
        assert_eq!(encode_key(key(KeyCode::Left)).unwrap(), b"\x1b[D");
    }

    #[test]
    fn alt_prefixes_with_escape() {
        let alt_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(encode_key(alt_a).unwrap(), b"\x1ba");
    }

    #[test]
    fn alt_prefixes_keys_with_no_parameterised_csi_form() {
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)).unwrap(),
            vec![0x1b, 0x7f]
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)).unwrap(),
            b"\x1b\r"
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT)).unwrap(),
            vec![0x1b, 0x1b]
        );
    }

    #[test]
    fn alt_on_keys_with_a_parameterised_csi_form_uses_the_modifier_not_an_escape_prefix() {
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT)).unwrap(),
            b"\x1b[1;3D"
        );
    }

    #[test]
    fn unsupported_keys_return_none() {
        assert!(encode_key(key(KeyCode::F(13))).is_none());
    }

    fn modified(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn unmodified_named_keys_keep_their_encodings() {
        assert_eq!(encode_key(key(KeyCode::BackTab)).unwrap(), b"\x1b[Z");
        assert_eq!(encode_key(key(KeyCode::Home)).unwrap(), b"\x1b[H");
        assert_eq!(encode_key(key(KeyCode::End)).unwrap(), b"\x1b[F");
        assert_eq!(encode_key(key(KeyCode::PageUp)).unwrap(), b"\x1b[5~");
        assert_eq!(encode_key(key(KeyCode::PageDown)).unwrap(), b"\x1b[6~");
        assert_eq!(encode_key(key(KeyCode::Insert)).unwrap(), b"\x1b[2~");
        assert_eq!(encode_key(key(KeyCode::Delete)).unwrap(), b"\x1b[3~");
        assert_eq!(encode_key(key(KeyCode::F(1))).unwrap(), b"\x1bOP");
        assert_eq!(encode_key(key(KeyCode::F(2))).unwrap(), b"\x1bOQ");
        assert_eq!(encode_key(key(KeyCode::F(3))).unwrap(), b"\x1bOR");
        assert_eq!(encode_key(key(KeyCode::F(4))).unwrap(), b"\x1bOS");
        assert_eq!(encode_key(key(KeyCode::F(5))).unwrap(), b"\x1b[15~");
        assert_eq!(encode_key(key(KeyCode::F(6))).unwrap(), b"\x1b[17~");
        assert_eq!(encode_key(key(KeyCode::F(7))).unwrap(), b"\x1b[18~");
        assert_eq!(encode_key(key(KeyCode::F(8))).unwrap(), b"\x1b[19~");
        assert_eq!(encode_key(key(KeyCode::F(9))).unwrap(), b"\x1b[20~");
        assert_eq!(encode_key(key(KeyCode::F(10))).unwrap(), b"\x1b[21~");
        assert_eq!(encode_key(key(KeyCode::F(11))).unwrap(), b"\x1b[23~");
        assert_eq!(encode_key(key(KeyCode::F(12))).unwrap(), b"\x1b[24~");
    }

    #[test]
    fn ctrl_arrows_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Left, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Right, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5C"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Up, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5A"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Down, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5B"
        );
    }

    #[test]
    fn alt_arrows_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Up, KeyModifiers::ALT)).unwrap(),
            b"\x1b[1;3A"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Left, KeyModifiers::ALT)).unwrap(),
            b"\x1b[1;3D"
        );
    }

    #[test]
    fn shift_arrows_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Up, KeyModifiers::SHIFT)).unwrap(),
            b"\x1b[1;2A"
        );
    }

    #[test]
    fn combined_modifiers_sum_into_the_parameter() {
        let ctrl_alt_shift = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT;
        assert_eq!(
            encode_key(modified(KeyCode::Up, ctrl_alt_shift)).unwrap(),
            b"\x1b[1;8A"
        );
    }

    #[test]
    fn modified_home_and_end_use_the_xterm_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::Home, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5H"
        );
        assert_eq!(
            encode_key(modified(KeyCode::End, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5F"
        );
    }

    #[test]
    fn modified_tilde_keys_add_the_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::PageUp, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[5;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::PageDown, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[6;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Insert, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[2;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::Delete, KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[3;5~"
        );
    }

    #[test]
    fn modified_low_function_keys_use_the_csi_form() {
        assert_eq!(
            encode_key(modified(KeyCode::F(1), KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[1;5P"
        );
        assert_eq!(
            encode_key(modified(KeyCode::F(4), KeyModifiers::ALT)).unwrap(),
            b"\x1b[1;3S"
        );
    }

    #[test]
    fn modified_high_function_keys_add_the_modifier_parameter() {
        assert_eq!(
            encode_key(modified(KeyCode::F(5), KeyModifiers::CONTROL)).unwrap(),
            b"\x1b[15;5~"
        );
        assert_eq!(
            encode_key(modified(KeyCode::F(12), KeyModifiers::ALT)).unwrap(),
            b"\x1b[24;3~"
        );
    }

    #[test]
    fn key_spec_builds_control_and_meta_specs() {
        use super::key_spec;

        let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(key_spec(&ctrl_x).as_deref(), Some("C-x"));

        let alt_left = modified(KeyCode::Left, KeyModifiers::ALT);
        assert_eq!(key_spec(&alt_left).as_deref(), Some("M-Left"));

        let dot = key(KeyCode::Char('.'));
        assert_eq!(key_spec(&dot).as_deref(), Some("."));
    }

    #[test]
    fn key_spec_is_none_for_release_events() {
        use super::key_spec;
        use crossterm::event::KeyEventKind;

        let released = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(key_spec(&released), None);
    }

    #[test]
    fn only_press_and_repeat_produce_bytes() {
        use crossterm::event::KeyEventKind;

        let released = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert!(encode_key(released).is_none());

        let repeated =
            KeyEvent::new_with_kind(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Repeat);
        assert_eq!(encode_key(repeated).unwrap(), b"a");
    }
}

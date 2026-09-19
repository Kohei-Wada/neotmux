use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn encode_key(key: KeyEvent) -> Option<Vec<u8>> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

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
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::F(n @ 1..=4) => vec![0x1b, b'O', b'P' + (n - 1)],
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
            format!("\x1b[{code}~").into_bytes()
        }
        _ => return None,
    };

    if alt {
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
    fn unsupported_keys_return_none() {
        assert!(encode_key(key(KeyCode::F(13))).is_none());
    }
}

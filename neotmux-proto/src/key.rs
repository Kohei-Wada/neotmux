//! The keyboard as neotmux understands it: the key forms the program binds and types into a
//! pane, and nothing else. There is no mouse, no bracketed paste and no focus event here
//! because nothing in neotmux asks for one.

use crate::KeyCode;
use crate::Modifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub mods: Modifiers,
}

impl Key {
    pub fn new(code: KeyCode, mods: Modifiers) -> Self {
        Self { code, mods }
    }

    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, Modifiers::NONE)
    }

    /// The key a spec names, or `None` for one nothing could mean.
    ///
    /// The inverse of the spec a plugin is handed for a key: `on_key` says `C-b` and this is
    /// what turns that back into the keystroke, so a plugin relaying a key into a pane sends
    /// what the person actually pressed. It lives here rather than beside either end because
    /// both ends need it — the Lua layer to refuse a spec nobody could have meant, and the
    /// host to encode one.
    ///
    /// Modifier prefixes are `C-`, `M-`/`A-` and `S-`, in any order, spelled out or not. A
    /// shift on a character is refused rather than guessed at: `S-a` is not `A`, and a
    /// terminal reports the character it produced.
    pub fn from_spec(spec: &str) -> Option<Self> {
        let mut mods = Modifiers::NONE;
        let mut rest = spec;
        while let Some((token, tail)) = rest.split_once('-') {
            match token.to_ascii_lowercase().as_str() {
                "c" | "ctrl" | "control" => mods.ctrl = true,
                "m" | "meta" | "alt" | "a" => mods.alt = true,
                "s" | "shift" => mods.shift = true,
                _ => break,
            }
            rest = tail;
        }
        if rest.is_empty() {
            return None;
        }
        let code = if rest.chars().count() == 1 {
            let c = rest.chars().next().expect("one character");
            if mods.shift {
                return None;
            }
            KeyCode::Char(c)
        } else if let Some(n) = rest.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
            KeyCode::F(n)
        } else {
            match rest {
                "Enter" => KeyCode::Enter,
                "Tab" => KeyCode::Tab,
                "BackTab" => KeyCode::BackTab,
                "Backspace" | "BS" => KeyCode::Backspace,
                "Esc" => KeyCode::Esc,
                "Up" => KeyCode::Up,
                "Down" => KeyCode::Down,
                "Left" => KeyCode::Left,
                "Right" => KeyCode::Right,
                "Home" => KeyCode::Home,
                "End" => KeyCode::End,
                "PageUp" => KeyCode::PageUp,
                "PageDown" => KeyCode::PageDown,
                "Insert" => KeyCode::Insert,
                "Delete" => KeyCode::Delete,
                _ => return None,
            }
        };
        Some(Self::new(code, mods))
    }

    pub fn with_alt(mut self) -> Self {
        self.mods.alt = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_names_the_key_it_was_built_from() {
        for (spec, key) in [
            ("a", Key::plain(KeyCode::Char('a'))),
            ("Enter", Key::plain(KeyCode::Enter)),
            ("BS", Key::plain(KeyCode::Backspace)),
            ("F5", Key::plain(KeyCode::F(5))),
            (
                "C-b",
                Key::new(
                    KeyCode::Char('b'),
                    Modifiers {
                        ctrl: true,
                        ..Modifiers::NONE
                    },
                ),
            ),
            (
                "M-Left",
                Key::new(
                    KeyCode::Left,
                    Modifiers {
                        alt: true,
                        ..Modifiers::NONE
                    },
                ),
            ),
            (
                "S-Tab",
                Key::new(
                    KeyCode::Tab,
                    Modifiers {
                        shift: true,
                        ..Modifiers::NONE
                    },
                ),
            ),
        ] {
            assert_eq!(Key::from_spec(spec), Some(key), "{spec}");
        }
    }

    /// A shifted character is the character the terminal produced, so saying it twice is a
    /// spec nobody could have meant rather than one to guess at.
    #[test]
    fn a_shift_on_a_character_is_refused() {
        assert_eq!(Key::from_spec("S-a"), None);
    }

    #[test]
    fn a_spec_naming_no_key_is_refused() {
        // `F` on its own is the letter, not a function key with no number — one character is
        // always the character.
        for spec in ["", "C-", "Nonsense", "F12x"] {
            assert_eq!(Key::from_spec(spec), None, "{spec}");
        }
    }
}

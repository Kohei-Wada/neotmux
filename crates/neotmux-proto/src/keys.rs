//! The keyboard as neotmux understands it: the key forms the program binds and types into a
//! pane, and nothing else. There is no mouse, no bracketed paste and no focus event here
//! because nothing in neotmux asks for one.

/// One key, named rather than encoded. `Char` carries the character a terminal sent, so a
/// UTF-8 key is one `Char` and not a run of bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

/// The three modifiers a terminal can report without a protocol extension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        alt: false,
        ctrl: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };

    /// xterm's modifier parameter: one plus a bitmask of shift (1), alt (2) and control (4).
    /// A parameter below one is the absent one, which is no modifiers.
    pub fn from_param(param: u16) -> Self {
        let bits = param.saturating_sub(1);
        Self {
            shift: bits & 1 != 0,
            alt: bits & 2 != 0,
            ctrl: bits & 4 != 0,
        }
    }

    /// The inverse of `from_param`, which is what an encoded `CSI 1;5D` is built from.
    pub fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

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
    fn the_modifier_parameter_round_trips() {
        for param in 1..=8u16 {
            assert_eq!(Modifiers::from_param(param).param(), param as u8);
        }
    }

    #[test]
    fn an_absent_modifier_parameter_is_no_modifiers() {
        assert_eq!(Modifiers::from_param(0), Modifiers::NONE);
        assert_eq!(Modifiers::from_param(1), Modifiers::NONE);
    }

    #[test]
    fn the_parameter_bits_are_shift_alt_control() {
        assert_eq!(Modifiers::from_param(2), Modifiers::SHIFT);
        assert_eq!(Modifiers::from_param(3), Modifiers::ALT);
        assert_eq!(Modifiers::from_param(5), Modifiers::CTRL);
        assert_eq!(
            Modifiers::from_param(8),
            Modifiers {
                shift: true,
                alt: true,
                ctrl: true
            }
        );
    }
}

#[cfg(test)]
mod spec_tests {
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

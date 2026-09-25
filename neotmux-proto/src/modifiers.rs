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

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    pub fn area(self) -> usize {
        usize::from(self.rows) * usize::from(self.cols)
    }
}

/// The orientation of a split, as §6's tree will read it. M1a only carries it across the
/// ABI; nothing splits yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

impl SplitDir {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "horizontal" => Some(Self::Horizontal),
            "vertical" => Some(Self::Vertical),
            _ => None,
        }
    }
}

impl fmt::Display for SplitDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_direction_round_trips_through_its_spelling() {
        for dir in [SplitDir::Horizontal, SplitDir::Vertical] {
            assert_eq!(SplitDir::parse(&dir.to_string()), Some(dir));
        }
    }

    #[test]
    fn an_unknown_spelling_is_not_a_direction() {
        assert_eq!(SplitDir::parse("sideways"), None);
        assert_eq!(SplitDir::parse("Horizontal"), None);
    }
}

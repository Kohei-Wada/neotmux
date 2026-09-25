use std::fmt;

/// The orientation of a split, as §6's tree will read it. M1a only carries it across the
/// ABI; nothing splits yet.
/// Not `#[non_exhaustive]`: a split is along one axis or the other, a decision rather than a
/// list waiting to grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

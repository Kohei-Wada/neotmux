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

/// Where a neighbour is, and therefore which boundary a resize moves. A split's orientation
/// answers to two of these: a vertical split's boundary is the one a left or right move
/// touches.
/// Not `#[non_exhaustive]`: four directions on a grid, and a fifth would have to break every
/// match that walks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            _ => None,
        }
    }

    /// The orientation of the splits whose boundary this direction crosses.
    pub fn orientation(self) -> SplitDir {
        match self {
            Self::Left | Self::Right => SplitDir::Vertical,
            Self::Up | Self::Down => SplitDir::Horizontal,
        }
    }

    /// Whether a pane moving its boundary this way is the *first* child of that split —
    /// the side the boundary has to be on for it to be the one between the pane and its
    /// neighbour in this direction.
    pub fn from_first_child(self) -> bool {
        matches!(self, Self::Right | Self::Down)
    }
}

impl fmt::Display for Dir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
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
    fn a_direction_round_trips_through_its_spelling_and_knows_its_orientation() {
        for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
            assert_eq!(Dir::parse(&dir.to_string()), Some(dir));
        }
        assert_eq!(Dir::Left.orientation(), SplitDir::Vertical);
        assert_eq!(Dir::Down.orientation(), SplitDir::Horizontal);
        assert!(Dir::Right.from_first_child());
        assert!(!Dir::Up.from_first_child());
        assert_eq!(Dir::parse("sideways"), None);
    }

    #[test]
    fn an_unknown_spelling_is_not_a_direction() {
        assert_eq!(SplitDir::parse("sideways"), None);
        assert_eq!(SplitDir::parse("Horizontal"), None);
    }
}

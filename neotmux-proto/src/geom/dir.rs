use std::fmt;

use crate::geom::SplitDir;

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
    use crate::geom::SplitDir;

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

use crate::geom::SplitDir;
use std::fmt;

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

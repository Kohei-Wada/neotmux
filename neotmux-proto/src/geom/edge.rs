use crate::geom::Axis;
use std::fmt;

/// Which edge of the surface a band is taken from.
///
/// Not `#[non_exhaustive]`: the set of edges is a decision, not a list waiting to grow, and
/// every match on it should stop compiling if that ever changes. It did change once — `left`
/// and `right` were absent until a sidebar wanted them — and walking the broken matches is
/// what made the column case explicit everywhere the row case already was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    pub fn axis(self) -> Axis {
        match self {
            Self::Top | Self::Bottom => Axis::Rows,
            Self::Left | Self::Right => Axis::Columns,
        }
    }
}

impl fmt::Display for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        })
    }
}

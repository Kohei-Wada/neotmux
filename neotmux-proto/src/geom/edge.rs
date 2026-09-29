use std::fmt;

use crate::geom::Axis;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::fixtures::EDGES;

    #[test]
    fn an_edge_round_trips_through_its_spelling() {
        for edge in EDGES {
            assert_eq!(Edge::parse(&edge.to_string()), Some(edge));
        }
    }

    #[test]
    fn a_name_no_edge_has_is_not_an_edge() {
        assert_eq!(Edge::parse("middle"), None);
        assert_eq!(Edge::parse("Top"), None);
    }

    #[test]
    fn a_horizontal_edge_is_rows_and_a_vertical_one_is_columns() {
        assert_eq!(Edge::Top.axis(), Axis::Rows);
        assert_eq!(Edge::Bottom.axis(), Axis::Rows);
        assert_eq!(Edge::Left.axis(), Axis::Columns);
        assert_eq!(Edge::Right.axis(), Axis::Columns);
    }
}

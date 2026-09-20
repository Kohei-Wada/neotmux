use std::fmt;

/// Which edge of the surface a band is taken from.
///
/// `left` and `right` are deliberately absent. The consumers are a status line at the bottom
/// and a window list at the top, both horizontal bands measured in rows; a vertical band is a
/// different axis with a different unit and no consumer today. The omission is a decision,
/// not an oversight.
///
/// Not `#[non_exhaustive]`: two edges is the decision above, not a list waiting to grow, and
/// every match on it should stop compiling if that ever changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

impl Edge {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }
}

impl fmt::Display for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        })
    }
}

/// How many rows of the surface each edge is reserved for. The host subtracts them from what
/// the panes get and never learns what goes in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Reserved {
    pub top: u16,
    pub bottom: u16,
}

impl Reserved {
    pub fn new(top: u16, bottom: u16) -> Self {
        Self { top, bottom }
    }

    pub fn rows(self) -> u16 {
        self.top.saturating_add(self.bottom)
    }

    pub fn of(self, edge: Edge) -> u16 {
        match edge {
            Edge::Top => self.top,
            Edge::Bottom => self.bottom,
        }
    }

    pub fn add(&mut self, edge: Edge, rows: u16) {
        match edge {
            Edge::Top => self.top = self.top.saturating_add(rows),
            Edge::Bottom => self.bottom = self.bottom.saturating_add(rows),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edge_round_trips_through_its_spelling() {
        for edge in [Edge::Top, Edge::Bottom] {
            assert_eq!(Edge::parse(&edge.to_string()), Some(edge));
        }
    }

    #[test]
    fn a_vertical_edge_is_not_an_edge() {
        assert_eq!(Edge::parse("left"), None);
        assert_eq!(Edge::parse("right"), None);
        assert_eq!(Edge::parse("Top"), None);
    }

    #[test]
    fn a_reservation_counts_the_rows_of_both_edges() {
        let mut reserved = Reserved::default();
        reserved.add(Edge::Top, 2);
        reserved.add(Edge::Bottom, 1);
        assert_eq!(reserved.of(Edge::Top), 2);
        assert_eq!(reserved.of(Edge::Bottom), 1);
        assert_eq!(reserved.rows(), 3);
    }
}

use crate::geom::Axis;
use crate::geom::Edge;

/// How much of the surface each edge is reserved for. The host subtracts it from what the
/// panes get and never learns what goes in it.
///
/// One type and not two, although the two axes have different units: what the host holds is
/// one surface with one set of bands, and splitting it would make every signature that
/// carries a reservation carry a pair of them. The units are kept honest by the totals
/// instead — [`rows`] adds up the horizontal edges and [`cols`] the vertical ones, and
/// neither can be read as the other.
///
/// [`rows`]: Reserved::rows
/// [`cols`]: Reserved::cols
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Reserved {
    pub top: u16,
    pub bottom: u16,
    pub left: u16,
    pub right: u16,
}

impl Reserved {
    pub fn horizontal(top: u16, bottom: u16) -> Self {
        Self {
            top,
            bottom,
            left: 0,
            right: 0,
        }
    }

    pub fn vertical(left: u16, right: u16) -> Self {
        Self {
            top: 0,
            bottom: 0,
            left,
            right,
        }
    }

    /// The rows the horizontal bands cost the panes.
    pub fn rows(self) -> u16 {
        self.top.saturating_add(self.bottom)
    }

    /// The columns the vertical bands cost the panes.
    pub fn cols(self) -> u16 {
        self.left.saturating_add(self.right)
    }

    /// The total along one axis, which is the one an edge's own band is measured against.
    pub fn along(self, axis: Axis) -> u16 {
        match axis {
            Axis::Rows => self.rows(),
            Axis::Columns => self.cols(),
        }
    }

    pub fn of(self, edge: Edge) -> u16 {
        match edge {
            Edge::Top => self.top,
            Edge::Bottom => self.bottom,
            Edge::Left => self.left,
            Edge::Right => self.right,
        }
    }

    pub fn add(&mut self, edge: Edge, cells: u16) {
        let slot = match edge {
            Edge::Top => &mut self.top,
            Edge::Bottom => &mut self.bottom,
            Edge::Left => &mut self.left,
            Edge::Right => &mut self.right,
        };
        *slot = slot.saturating_add(cells);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::geom::fixtures::EDGES;

    #[test]
    fn each_edge_keeps_its_own_band() {
        let mut reserved = Reserved::default();
        for (n, edge) in EDGES.iter().enumerate() {
            reserved.add(*edge, n as u16 + 1);
        }
        for (n, edge) in EDGES.iter().enumerate() {
            assert_eq!(reserved.of(*edge), n as u16 + 1);
        }
    }

    #[test]
    fn the_two_totals_count_their_own_axis_and_nothing_else() {
        let mut reserved = Reserved::horizontal(2, 1);
        reserved.add(Edge::Left, 20);
        assert_eq!(reserved.rows(), 3);
        assert_eq!(reserved.cols(), 20);
        assert_eq!(reserved.along(Axis::Rows), 3);
        assert_eq!(reserved.along(Axis::Columns), 20);
    }

    #[test]
    fn a_band_on_one_axis_leaves_the_other_at_zero() {
        assert_eq!(Reserved::vertical(20, 0).rows(), 0);
        assert_eq!(Reserved::horizontal(1, 0).cols(), 0);
    }
}

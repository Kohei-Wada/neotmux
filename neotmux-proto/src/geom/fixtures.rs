//! Test helpers shared by `edge.rs`'s and `reserved.rs`'s tests.

use crate::geom::Edge;

pub(crate) const EDGES: [Edge; 4] = [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right];

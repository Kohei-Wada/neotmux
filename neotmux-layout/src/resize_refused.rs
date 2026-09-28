use std::error::Error;
use std::fmt;

/// Why a resize did not happen. The tree is unchanged in every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeRefused {
    /// The target pane is not in this tree.
    UnknownPane,
    /// No other pane shares the axis, so there is no boundary to move that way.
    NoNeighbour,
    /// One of the two sides would be smaller than the caller's minimum.
    TooSmall,
}

impl fmt::Display for ResizeRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::UnknownPane => "no such pane",
            Self::NoNeighbour => "no neighbour that way",
            Self::TooSmall => "the panes would be too small",
        };
        f.write_str(reason)
    }
}

impl Error for ResizeRefused {}

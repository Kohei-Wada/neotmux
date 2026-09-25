use std::fmt;

/// The axis a band is measured along, and so the unit it is counted in.
///
/// Two axes is the whole of why there are four edges: a band at the top or the bottom is a
/// number of rows, one at the left or the right a number of columns. Nothing else about a
/// reservation differs between them, so everything that has to say "rows" or "columns" —
/// a bound, a message, a total — asks the edge for its axis instead of matching on it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Columns,
}

impl fmt::Display for Axis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rows => "rows",
            Self::Columns => "columns",
        })
    }
}

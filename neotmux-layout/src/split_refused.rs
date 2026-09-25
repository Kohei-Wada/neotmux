use std::error::Error;
use std::fmt;

/// Why a split did not happen. The tree is unchanged in both cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRefused {
    /// The target pane is not in this tree.
    UnknownPane,
    /// One of the two children would be smaller than the caller's minimum.
    TooSmall,
}

impl fmt::Display for SplitRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::UnknownPane => "no such pane",
            Self::TooSmall => "the panes would be too small",
        };
        f.write_str(reason)
    }
}

impl Error for SplitRefused {}

//! What a caller asks of a read of a pane's text, and what it gets back.
//!
//! Two structs rather than two more positional arguments, because every reader of a pane
//! wants a different subset of the same read and the alternative is a signature that grows
//! a parameter and a tuple slot every time one of them wants one more thing.

use crate::lines::Trailing;

/// What to include in a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// What to do with the blank rest of each row.
    pub trailing: Trailing,
    /// Whether to also report, per row, if its text runs on into the next one. Off by
    /// default: a reader that does not need it should not pay for the pass.
    pub wrapped: bool,
}

/// One read of a pane's text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub lines: Vec<String>,
    /// Whether each line runs on into the next, the same length as `lines` — or `None`
    /// where it was not asked for. `None` is "not asked", never "none of them wrap".
    pub wrapped: Option<Vec<bool>>,
    /// The revision the read was taken at.
    pub revision: u64,
}

/// Where a pane's history begins, and how long it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// How many lines the pane holds above its visible screen.
    pub len: usize,
    /// How many lines have fallen out of the front of it since the pane opened, so history
    /// line `i` is the `dropped + i`th line the pane ever scrolled away. See
    /// [`crate::evictions`] for the one case in which this can be low.
    pub dropped: usize,
}

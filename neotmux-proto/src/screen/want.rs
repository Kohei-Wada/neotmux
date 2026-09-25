use crate::screen::Trailing;

/// What to include in a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// What to do with the blank rest of each row.
    pub trailing: Trailing,
    /// Whether to also report, per row, if its text runs on into the next one. Off by
    /// default: a reader that does not need it should not pay for the pass.
    pub wrapped: bool,
}

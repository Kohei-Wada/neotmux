use crate::screen::Trailing;

/// What to include in a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// What to do with the blank rest of each row.
    pub trailing: Trailing,
    /// Whether to also report, per row, if its text runs on into the next one. Off by
    /// default: a reader that does not need it should not pay for the pass.
    pub wrapped: bool,
    /// Whether to also answer each row as [`crate::screen::Run`]s, with the colours and
    /// attributes the program drew it in. Off by default for the same reason.
    pub runs: bool,
}

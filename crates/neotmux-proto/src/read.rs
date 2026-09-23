//! Asking a pane for its text, and what comes back.
//!
//! One vocabulary, not two. These are the types the Lua layer names, the types
//! [`crate::msg::Read`] and [`crate::msg::Reading`] carry across the socket, and the types
//! the local terminals answer in — which is §2.1's rule about the frame ("one definition
//! each, no conversion layer") applied to the reads, where it had not been.
//!
//! [`PaneRead`] is here rather than beside either implementation because a contract belongs
//! in the lowest crate that both its consumer and every one of its implementors can see. The
//! consumer is the Lua layer; the implementors are the session's own terminals and, in the
//! client, a stand-in that asks the socket. Nothing below this crate is common to all three.

use crate::PaneId;

/// What to do with the blank rest of a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trailing {
    /// Cut it, so an untouched row is `""`. What reading a line of text wants.
    #[default]
    Trim,
    /// Keep it, so every row is exactly as wide as the pane. What a rectangle wants: the
    /// interior spacing of a short line is where its columns are.
    Pad,
}

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
pub struct Lines {
    pub lines: Vec<String>,
    /// Whether each line runs on into the next, the same length as `lines` — or `None`
    /// where it was not asked for. `None` is "not asked", never "none of them wrap".
    pub wrapped: Option<Vec<bool>>,
    /// The revision the read was taken at.
    pub revision: u64,
}

/// Where a pane's history begins, and how long it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extent {
    /// How many lines the pane holds above its visible screen.
    pub len: u64,
    /// How many lines have fallen out of the front of it since the pane opened, so history
    /// line `i` is the `dropped + i`th line the pane ever scrolled away.
    pub dropped: u64,
}

/// Reading a pane's text, from wherever the pane's terminal happens to be.
///
/// `neotmux-term`'s `PaneTerminals` is the local answer, and the one the session holds. The
/// Lua layer runs in the process with the screen rather than the one with the panes, so its
/// reads are answered across a socket instead — same questions, a different distance, and
/// since M4 the same types either way.
pub trait PaneRead {
    fn revision(&self, pane: PaneId) -> Option<u64>;
    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines>;
    fn history_extent(&self, pane: PaneId) -> Option<Extent>;
    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines>;
    fn freeze(&self, pane: PaneId) -> bool;
    fn unfreeze(&self, pane: PaneId) -> bool;
    fn is_frozen(&self, pane: PaneId) -> Option<bool>;
}

//! Asking a pane for its text, and what comes back.
//!
//! One vocabulary, not two. These are the types the Lua layer names and the types the
//! terminals answer in — which is §2.1's rule about the frame ("one definition each, no
//! conversion layer") applied to the reads, where it had not been.
//!
//! [`PaneRead`] is here rather than beside its implementation because a contract belongs in
//! the lowest crate that both its consumer and its implementor can see. The consumer is the
//! Lua layer; the implementor is the session's own terminals. Nothing below this crate is
//! common to both.

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
    /// line `i` is the `dropped + i`th line the pane ever scrolled away. A single feed that
    /// appends a whole ring's worth of lines in one go can undercount this (pinned by
    /// `a_feed_that_appends_a_whole_ring_undercounts` in `neotmux-term`), so a plugin must
    /// not treat it as exact.
    pub dropped: u64,
}

/// Reading a pane's text.
///
/// `neotmux-term`'s `PaneTerminals` answers it, and the session holds it. The Lua layer runs
/// in the server, beside the panes, so a read is a function call; the trait is what keeps
/// the Lua crate from having to know a terminal.
pub trait PaneRead {
    fn revision(&self, pane: PaneId) -> Option<u64>;
    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines>;
    fn history_extent(&self, pane: PaneId) -> Option<Extent>;
    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines>;
    fn freeze(&self, pane: PaneId) -> bool;
    fn unfreeze(&self, pane: PaneId) -> bool;
    fn is_frozen(&self, pane: PaneId) -> Option<bool>;
}

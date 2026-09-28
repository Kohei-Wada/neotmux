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
use crate::screen::Extent;
use crate::screen::Hold;
use crate::screen::Lines;
use crate::screen::Mark;
use crate::screen::Want;

/// Reading a pane's text.
///
/// `neotmux-pty`'s `PaneTerminals` answers it, and the session holds it. The Lua layer runs
/// in the server, beside the panes, so a read is a function call; the trait is what keeps
/// the Lua crate from having to know a terminal.
pub trait PaneRead {
    fn revision(&self, pane: PaneId) -> Option<u64>;
    /// The title the pane's program set on itself — the program's own claim, not anything
    /// read out of `/proc`. `None` for a pane that never set one, or that does not exist.
    fn title(&self, pane: PaneId) -> Option<String>;
    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines>;
    fn history_extent(&self, pane: PaneId) -> Option<Extent>;
    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines>;
    /// The OSC 133 marks the pane's shell sent on lines `first..=last`, in line order. Empty
    /// for a shell that sends none; `None` for a pane that does not exist.
    fn marks(&self, pane: PaneId, first: u64, last: u64) -> Option<Vec<Mark>>;
    fn is_frozen(&self, pane: PaneId) -> Option<bool>;
    /// Holds the pane still until the hold is released: it draws and reads as it was when
    /// the first hold on it was taken. `None` for a pane that does not exist.
    fn hold(&self, pane: PaneId) -> Option<Hold>;
    /// `false` if this hold was already released or its pane is gone.
    fn release(&self, hold: Hold) -> bool;
}

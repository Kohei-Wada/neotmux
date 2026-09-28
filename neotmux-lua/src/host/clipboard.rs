//! What the host owes a plugin that wants to put something on the clipboard.
//!
//! The Lua layer has no terminal. It runs beside one — M3 puts it in the client — but the
//! handle, and the escape sequence that goes down it, are the host's, the same way a pane's
//! text is the host's and reaches Lua through [`neotmux_proto::screen::PaneRead`]. A contract
//! rather than a write is what keeps this crate from knowing either.
//!
//! Declared here rather than in `neotmux-proto`, because the only implementor is the client
//! binary, which already depends on this crate: a contract belongs in the lowest crate that
//! both its consumer and every one of its implementors can see, and for this one that is
//! here. Nothing puts it on the wire; the client writes to its own terminal.

use neotmux_proto::SessionId;

/// Somewhere to put text a plugin has yanked.
pub trait Clipboard {
    /// Hands `text` over. `Ok` means the host did what it does — for the client that is
    /// writing the sequence to its terminal and flushing it — and never that a clipboard now
    /// holds the text, which nothing in this direction can know.
    ///
    /// `session` is the one the yank was raised for — the session whose tick, key or event
    /// the plugin was answering — so that it reaches the terminals looking at that session and
    /// no other. `None` is a yank from outside any of them, a config load.
    fn set(&self, session: Option<SessionId>, text: &str) -> std::io::Result<()>;
}

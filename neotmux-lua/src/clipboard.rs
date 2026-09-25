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

/// Somewhere to put text a plugin has yanked.
pub trait Clipboard {
    /// Hands `text` over. `Ok` means the host did what it does — for the client that is
    /// writing the sequence to its terminal and flushing it — and never that a clipboard now
    /// holds the text, which nothing in this direction can know.
    fn set(&self, text: &str) -> std::io::Result<()>;
}

/// The one a subsystem built with [`crate::LuaSubsystem::build`] gets: a host that has no
/// clipboard at all, which answers so rather than claiming a write it did not make.
pub struct Absent;

impl Clipboard for Absent {
    fn set(&self, _text: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("this host has no clipboard"))
    }
}

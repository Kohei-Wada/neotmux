//! Where a yank waits, now that the Lua layer is in the process with no terminal.
//!
//! OSC 52 is a sequence written to a terminal, and the server's terminals are its clients'.
//! So `neotmux.clipboard.set` queues the text here and the frame hands it to whoever is
//! attached, which is the one place that knows who that is.

use std::cell::{Cell, RefCell};

#[derive(Default)]
pub struct Clip {
    queued: RefCell<Vec<String>>,
    attached: Cell<usize>,
}

impl Clip {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many clients there are to write to, as of this frame.
    pub fn attached(&self, count: usize) {
        self.attached.set(count);
    }

    /// Everything yanked since the last frame took it.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.queued.borrow_mut())
    }
}

impl neotmux_lua::clipboard::Clipboard for Clip {
    fn set(&self, text: &str) -> std::io::Result<()> {
        if self.attached.get() == 0 {
            return Err(std::io::Error::other(
                "no client is attached, so there is no terminal to write it to",
            ));
        }
        self.queued.borrow_mut().push(text.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neotmux_lua::clipboard::Clipboard as _;

    #[test]
    fn a_yank_with_a_client_attached_is_queued_once_and_taken_once() {
        let clip = Clip::new();
        clip.attached(1);
        clip.set("hello").expect("a client is attached");
        assert_eq!(clip.take(), vec!["hello".to_string()]);
        assert!(clip.take().is_empty(), "taking it twice sends it twice");
    }

    /// `Clipboard::set` promises the host did what it does. With no terminal to write to
    /// there is nothing it does, and saying so is the honest answer — the Lua side turns it
    /// into the `false` a plugin can act on.
    #[test]
    fn a_yank_with_nobody_attached_is_refused_rather_than_held() {
        let clip = Clip::new();
        clip.attached(0);
        assert!(clip.set("hello").is_err());
        assert!(clip.take().is_empty());
    }
}

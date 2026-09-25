use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use signal_hook::consts::SIGWINCH;

/// The terminal changing size. The kernel says so with `SIGWINCH` and with nothing else:
/// no byte arrives on the terminal to announce it, so a program that reads its own input
/// has to install this handler or never notice a resize at all.
pub struct WindowChange {
    changed: Arc<AtomicBool>,
}

impl WindowChange {
    pub fn install() -> anyhow::Result<Self> {
        let changed = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGWINCH, Arc::clone(&changed))?;
        Ok(Self { changed })
    }

    /// Whether the terminal has changed size since this was last asked. Taking the flag is
    /// what keeps one resize from being applied on every frame after it.
    pub fn taken(&self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_change_is_reported_once_and_then_not_again() {
        let winch = WindowChange::install().expect("install");
        assert!(!winch.taken(), "nothing has resized anything yet");

        winch.changed.store(true, Ordering::SeqCst);
        assert!(winch.taken());
        assert!(!winch.taken(), "one resize was applied twice");
    }
}

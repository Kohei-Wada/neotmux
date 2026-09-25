use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

pub struct ShutdownSignal {
    term: Arc<AtomicBool>,
    hup: Arc<AtomicBool>,
    int: Arc<AtomicBool>,
}

impl ShutdownSignal {
    pub fn install() -> anyhow::Result<Self> {
        let term = Arc::new(AtomicBool::new(false));
        let hup = Arc::new(AtomicBool::new(false));
        let int = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGTERM, Arc::clone(&term))?;
        signal_hook::flag::register(SIGHUP, Arc::clone(&hup))?;
        signal_hook::flag::register(SIGINT, Arc::clone(&int))?;
        Ok(Self { term, hup, int })
    }

    pub fn raised(&self) -> Option<i32> {
        if self.term.load(Ordering::SeqCst) {
            Some(SIGTERM)
        } else if self.hup.load(Ordering::SeqCst) {
            Some(SIGHUP)
        } else if self.int.load(Ordering::SeqCst) {
            Some(SIGINT)
        } else {
            None
        }
    }
}

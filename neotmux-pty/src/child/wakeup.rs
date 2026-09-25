use std::io::{self, PipeReader, Read as _};
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::Arc;

use crate::child::Waker;

/// The two ends of the nudge a pane's reader thread gives the loop that draws it: a
/// [`Waker`] per reader thread, and one [`Wakeup`] a wait can poll.
pub fn wakeup() -> io::Result<(Wakeup, Waker)> {
    let (reader, writer) = io::pipe()?;
    Ok((
        Wakeup { reader },
        Waker {
            writer: Arc::new(writer),
        },
    ))
}

/// The end a wait watches. A channel cannot be handed to `poll(2)`, so this pipe is what
/// makes "a pane produced output" as waitable as "the user typed": one byte per chunk a
/// reader thread enqueues, carrying no information beyond the fact that there is something
/// to drain.
pub struct Wakeup {
    reader: PipeReader,
}

impl Wakeup {
    /// Takes the nudges that piled up, so the same output does not wake the next wait as
    /// well. Called only once a poll has said the pipe is readable, which is what keeps the
    /// read from blocking; nudges past the buffer's worth simply wake the next wait, which
    /// clears them in turn.
    pub fn clear(&self) {
        let mut buf = [0u8; 256];
        let _ = (&self.reader).read(&mut buf);
    }
}

impl AsFd for Wakeup {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.reader.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_takes_every_nudge_that_piled_up() {
        let (wakeup, waker) = wakeup().expect("wakeup");
        for _ in 0..3 {
            waker.wake();
        }
        wakeup.clear();

        waker.wake();
        let mut buf = [0u8; 8];
        let read = (&wakeup.reader).read(&mut buf).expect("read");
        assert_eq!(read, 1, "clearing left {} earlier nudges behind", read - 1);
    }

    /// Shutdown drops the loop's end while the reader threads are still running; waking a
    /// wakeup nobody is waiting on has to be a no-op rather than a panic or a signal.
    #[test]
    fn waking_after_the_waiting_end_is_gone_is_harmless() {
        let (wakeup, waker) = wakeup().expect("wakeup");
        drop(wakeup);
        waker.wake();
    }
}

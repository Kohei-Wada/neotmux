use std::io::{PipeWriter, Write as _};
use std::sync::Arc;

/// The end a reader thread holds. Cloning it is how every pane comes to share one wakeup.
#[derive(Clone)]
pub struct Waker {
    pub(crate) writer: Arc<PipeWriter>,
}

impl Waker {
    /// Says there is something to drain. A failure here means the loop that was to be woken
    /// is gone, which is a session shutting down rather than anything a reader thread can do
    /// something about.
    pub fn wake(&self) {
        let _ = (&*self.writer).write(&[0]);
    }
}

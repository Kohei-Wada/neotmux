use crate::host::JobChunk;

/// A job that has started. Dropping it must not leave a process behind.
pub trait Running {
    /// Whatever has arrived since the last call, and nothing else. Never blocks: this is
    /// called from inside a frame, once per job, and a frame that waited on a child would be
    /// a frame every session waits for.
    fn poll(&mut self) -> Vec<JobChunk>;

    /// Ends it now, without waiting. Idempotent — the caller may already have seen
    /// [`JobChunk::Exited`], so one that has already exited is `Ok` — an error is a program
    /// that may still be running.
    fn kill(&mut self) -> std::io::Result<()>;
}

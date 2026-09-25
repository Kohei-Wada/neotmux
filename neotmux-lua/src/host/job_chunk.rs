/// One thing a running job has said. Bytes, not lines: where the line breaks fall is
/// [`crate::sched::Jobs`]'s business, and a chunk boundary is not a line boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobChunk {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// The program exited on its own, with this status.
    Exited(i32),
    /// It never got that far: the pipe broke, the child could not be waited for. One line.
    Failed(String),
}

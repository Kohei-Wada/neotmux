/// Where the buffer ended before a source's tick group ran, so that group's own ops can be
/// taken back out if its frame is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark(pub(crate) usize);

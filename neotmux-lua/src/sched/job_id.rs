/// What `job.start` answered with, and what `job:kill()` names. Minted here rather than by
/// the host for the reason `WindowIds` gives: the call has to answer before the frame ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct JobId(pub u64);

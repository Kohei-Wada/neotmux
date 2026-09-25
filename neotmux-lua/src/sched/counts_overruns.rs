/// What a job's and a timer's callbacks share once one has been measured: a call inside its
/// budget is forgiven, and one that overran is counted until the thing it belongs to is taken
/// away. What taking away means is each one's own — a job is killed and still owed its ending,
/// a timer is disarmed and owed nothing.
pub trait CountsOverruns {
    type Id: Copy;

    /// What the user is told was taken away, and how: `job` and `killed`, `timer` and
    /// `stopped`.
    const WHAT: &'static str;
    const ENDED: &'static str;

    fn forgive(&self, id: Self::Id);

    /// Counts one overrunning call. True only for the call that took it away, so the caller
    /// says so once.
    fn overran(&self, id: Self::Id, limit: u32) -> bool;
}

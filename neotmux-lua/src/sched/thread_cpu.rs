use std::time::Duration;

use nix::time::{ClockId, clock_gettime};

/// The CPU this thread has used since it started, or `None` where the clock cannot be read.
///
/// A system call, not a vDSO read like `Instant::now`, so `Budget` asks it only once the wall
/// clock has already passed a deadline: CPU time never runs ahead of wall time, and below the
/// deadline on the wall there is nothing to ask.
pub(crate) fn thread_cpu() -> Option<Duration> {
    clock_gettime(ClockId::CLOCK_THREAD_CPUTIME_ID)
        .ok()
        .map(Duration::from)
}

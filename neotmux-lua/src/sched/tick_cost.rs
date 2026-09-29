use std::collections::VecDeque;
use std::time::Duration;

/// What one source's ticks have cost so far: how many ran, their time together, and the
/// longest. Wall time around the source's whole group, host calls included, which is what a
/// frame waits on. `recent` is the last [`RECENT`] of them, oldest first, for a trend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickCost {
    pub ticks: u64,
    pub total: Duration,
    pub longest: Duration,
    pub recent: VecDeque<Duration>,
}

pub const RECENT: usize = 120;

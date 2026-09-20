use std::time::Duration;

/// How long one pass of the frame loop lasts: the tick's cadence, and the closest together
/// two draws can be. A frame no longer spends its remainder asleep — the loop pumps input
/// until the deadline instead — so the rate is still a property of the loop rather than of
/// whatever the input wait happens to do.
pub const FRAME: Duration = Duration::from_millis(16);

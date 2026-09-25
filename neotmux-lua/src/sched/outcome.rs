use std::time::Duration;

/// What one source's slice of one frame came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    WithinBudget,
    /// The hook fired past the deadline and unwound the tick.
    Interrupted,
    /// Past the deadline, but no sample ever landed there to cut it off: one long C call
    /// leaves almost no Lua behind it (§5.4). Attributable, not interruptible.
    RanLong {
        elapsed: Duration,
    },
}

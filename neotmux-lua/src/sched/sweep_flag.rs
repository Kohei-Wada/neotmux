use std::cell::Cell;

use crate::sched::Sweeping;

/// Whether a session is being swept, which is the one stretch of a frame where [`Jobs`] must
/// refuse a `start` and [`Timers`] an `arm`. What either would take on is bound to the dying
/// session — the ambient session is still bound inside its `on_exit` callbacks — and then swept
/// by nobody: a child left running with nothing to reap it, or a callback held for the life of
/// the server with no frame left to fire it.
///
/// [`Jobs`]: crate::sched::Jobs
/// [`Timers`]: crate::sched::timers::Timers
#[derive(Default)]
pub struct SweepFlag(Cell<bool>);

impl SweepFlag {
    /// On for as long as the guard lives.
    pub fn raise(&self) -> Sweeping<'_> {
        self.0.set(true);
        Sweeping(self)
    }

    pub fn is_raised(&self) -> bool {
        self.0.get()
    }

    pub(crate) fn lower(&self) {
        self.0.set(false);
    }
}

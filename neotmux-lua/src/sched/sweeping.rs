use crate::sched::SweepFlag;

/// Held for the length of a session sweep. A guard rather than a flag the caller clears,
/// because a callback that raises must not leave `start` or `arm` refusing for the rest of the
/// run.
pub struct Sweeping<'a>(pub(crate) &'a SweepFlag);

impl Drop for Sweeping<'_> {
    fn drop(&mut self) {
        self.0.lower();
    }
}

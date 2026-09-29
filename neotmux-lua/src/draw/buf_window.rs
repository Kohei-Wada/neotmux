use crate::draw::{FloatId, ReservationId};

/// What a buffer can be shown in: a band of the surface, or a float. `pub(crate)`, like
/// `FloatId` itself, rather than `pub`: a `pub` enum may not expose a less-visible variant
/// field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
// Constructed from Task 5, wiring `set_buf` on a reserve/float handle to `TextBuffers::show`.
#[allow(dead_code)]
pub(crate) enum BufWindow {
    Band(ReservationId),
    Float(FloatId),
}

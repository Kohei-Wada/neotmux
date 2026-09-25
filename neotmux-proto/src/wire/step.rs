use crate::{Event, Geometry, Reserved, Size};

/// One thing that happened to the session, in the order it happened.
///
/// A batch of events with one geometry attached would be a lie: a `pane_opened` handler must
/// see the pane it is being told about, and a `pane_closed` handler must not see the dead
/// one, so the geometry moves *between* the events exactly as it does when the two halves
/// are one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Geometry(Geometry),
    Bands { surface: Size, applied: Reserved },
    Event(Event),
}

use crate::wire::End;
use crate::wire::Step;

/// The end of a frame: what applying the tick's reply did to the session, the picture that
/// came of it, and whether that was the last one.
///
/// It is one message and not three because it is one moment. The client may not look at its
/// keyboard between sending a [`Tock`](crate::wire::Tock) and reading this: what a key handler sees has to be
/// the session as it is, and between those two points it is not — the ops have been applied
/// at the far end and nothing here knows it yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done<'a> {
    pub steps: Vec<Step>,
    /// `None` when nothing changed and the last frame still stands.
    pub frame: Option<&'a crate::Frame<'a>>,
    pub end: Option<End>,
}

use neotmux_proto::screen::Frame;
use neotmux_proto::wire::{End, Step};

/// What one pass of the session produced.
pub struct Pass<'a> {
    /// What applying the tick's reply did to the session, in order. It is answered in the
    /// same breath as the frame rather than held for the next tick: a key handler must see
    /// the session as it is, and between the reply and this it is not.
    pub steps: Vec<Step>,
    /// The frame to show, if this pass changed anything.
    pub frame: Option<Frame<'a>>,
    /// Why the session is over, if it is.
    pub end: Option<End>,
}

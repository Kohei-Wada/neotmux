use crate::wire::OwnedDone;
use crate::wire::Tick;

/// What a decoded [`ToClient`](crate::wire::ToClient) is held in, for the same reason [`crate::OwnedFrame`] exists:
/// the far end has nothing to borrow from until it has decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedToClient {
    Tick(Tick),
    Done(OwnedDone),
    Refused(String),
    Clip(String),
    Bye(Vec<String>),
    Sessions(Vec<String>),
    Killed(bool),
    Ran { ok: bool, lines: Vec<String> },
}

use crate::wire::End;

/// What a decoded [`Done`](crate::wire::Done) is held in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnedDone {
    pub frame: Option<crate::OwnedFrame>,
    pub end: Option<End>,
}

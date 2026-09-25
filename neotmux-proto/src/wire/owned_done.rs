use crate::wire::End;
use crate::wire::Step;

/// What a decoded [`Done`](crate::wire::Done) is held in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnedDone {
    pub steps: Vec<Step>,
    pub frame: Option<crate::OwnedFrame>,
    pub end: Option<End>,
}

use neotmux_proto::wire::{End, Step};

/// What one tock did to a session, without the picture it is shown in.
pub struct Apply {
    /// What applying the tock's ops did to the session, in order, for the reason `Pass`
    /// already gives it.
    pub steps: Vec<Step>,
    /// Why the session is over, if it is: a `Quit` a client's op raised, or an `Eof` the
    /// last of its shells just went.
    pub end: Option<End>,
}

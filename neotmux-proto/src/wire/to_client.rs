use crate::wire::Done;

/// Server to client. Borrowed, because the two big payloads — a frame's grids and a tick's
/// geometry — are held by the session and encoding writes straight through the borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToClient<'a> {
    Done(&'a Done<'a>),
    /// Why this connection is not getting a session: something else already has it.
    Refused(&'a str),
    /// Every session this server holds, in name order.
    Sessions(&'a [String]),
    /// Whether there was a session by that name to end.
    Killed(bool),
    /// Text a plugin yanked, for the client to put on its terminal's clipboard. It is the
    /// one thing the Lua layer does that needs a terminal, and the terminal is over here.
    Clip(&'a str),
    /// The notifications raised while this client was attached, for it to replay on stderr
    /// once its terminal is restored. Sent before the frame that carries the end.
    Bye(&'a [String]),
    /// What a command did. `lines` is stdout where `ok`, and the one sentence that says why
    /// not where it is not. Never both.
    Ran {
        ok: bool,
        lines: &'a [String],
    },
}

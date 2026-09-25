/// Why the client's conversation with the server is over. `Quit` and `Eof` are the session
/// ending too; `Detach` is the client alone leaving. A signal is none of these: it ends a
/// process, not the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Quit,
    Eof,
    /// The session is kept; only the client leaves.
    Detach,
}

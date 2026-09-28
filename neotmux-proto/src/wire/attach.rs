use crate::Size;

/// What the client tells the server when it takes the session over: how big its terminal is,
/// which is the one thing about the session only it knows.
///
/// It used to carry four more facts — the first window's id, the bands the plugins reserved,
/// the gutter and the `pane_output` subscription — because the Lua layer that knew them was
/// the client's. It is the server's now, and the server does not have to be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attach {
    pub size: Size,
    /// Which session this client is asking for. The server hands back the one with that name
    /// or makes it, which is what `neotmux <name>` has always meant. `None` is `neotmux` with
    /// no name: a new session, called the lowest number no running one is.
    pub session: Option<String>,
}

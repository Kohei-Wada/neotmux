use crate::Size;
use crate::wire::Attach;
use crate::wire::Tock;

/// Client to server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToServer {
    Attach(Attach),
    /// A key nothing on the client consumed, for whichever pane has the focus.
    Key(crate::Key),
    /// The terminal changed size. The client owns the terminal, so it owns `SIGWINCH`.
    Resize(Size),
    Tock(Tock),
    /// Not a client at all: a connection asking what sessions there are. The answer is the
    /// whole of what it wanted, and it is let go afterwards.
    List,
    /// Nor this one: end that session, whoever is attached to it.
    Kill {
        session: String,
    },
    /// Not a client either: a connection asking the server to run a named command. The
    /// answer is the whole of what it wanted, and it is let go afterwards.
    Command {
        /// The session the command is addressed to. `None` is the default name, the same
        /// rule an attach follows.
        session: Option<String>,
        name: String,
        args: Vec<String>,
    },
}

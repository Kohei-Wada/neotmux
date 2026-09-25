//! Both halves of neotmux, told apart by where they live.
//!
//! One executable is both. With no arguments it is a client, and it is the client — in
//! `client::start` — that re-execs it with `--server` when nothing is listening, and then
//! talks to whatever came up. [`server`] is the half that owns the sessions — the panes,
//! their children, the Lua layer beside them — and never draws. [`client`] is the half
//! that owns a terminal — it reads keys off it and paints the frames the server sends —
//! and owns no session. Between them is `link` over the socket [`sock`] says where to
//! find, carrying what `neotmux-proto` defines.
//!
//! What is left at this level is what both halves need and neither owns: the wire (`link`,
//! [`sock`]), the log ([`logging`]), the signals a process gets (`signals`), the waiting a
//! poll is — and, beside it, the file the client reads its keys from — (`tty`), and what
//! `/proc` says about a pid (`procinfo`).
//!
//! Where a new module goes is decided by who calls it: a session's business is the server's,
//! a terminal's is the client's, and something both halves call stays here. A module here
//! must never reach into either half — that is the direction that says the split has stopped
//! being true.

pub mod client;
pub mod server;

pub(crate) mod link;
pub mod logging;
pub(crate) mod procinfo;
pub(crate) mod signals;
pub mod sock;
pub(crate) mod tty;

// A frame, a key, the geometry and the events all live in `neotmux-proto`: they are what
// the wire carries.

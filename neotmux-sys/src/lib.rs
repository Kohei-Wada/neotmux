//! What both halves of neotmux need from the operating system and neither owns: where the socket
//! is and binding it ([`sock`]), the connection over it ([`link`]), the log ([`logging`]), the
//! signals a process gets ([`signals`]), the waiting a poll is — and, beside it, the file the
//! client reads its keys from — ([`tty`]), and what `/proc` says about a pid ([`procinfo`]).
//!
//! One executable is both halves. With no arguments it is a client (`neotmux-client`), and it is
//! the client that re-execs it with `--server` when nothing is listening. The server
//! (`neotmux-server`) owns the sessions and never draws; the client owns a terminal and owns no
//! session. Between them is [`link`] over the socket [`sock`] says where to find, carrying what
//! `neotmux-proto` defines.
//!
//! Where a new module goes is decided by who calls it: a session's business is the server's, a
//! terminal's is the client's, and something both halves call is here. Nothing here may reach
//! into either half — this crate cannot name them, which is the point.

pub mod link;
pub mod logging;
pub mod procinfo;
pub mod signals;
pub mod sock;
pub mod tty;

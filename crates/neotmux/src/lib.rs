//! Both halves of neotmux, with no terminal in either.
//!
//! [`session`] is the panes, the windows and the shells behind them; [`plugins`] is the Lua
//! layer and the notifications it raises. Both are the server's, in one process, so what is
//! between them is a function call: a tick out of one, its reply out of the other, and the
//! frame that comes of it. [`link`] and [`sock`] are the wire to the client that paints that
//! frame, and where it lives.
//!
//! Nothing here knows what it is being looked at through. A session is told how big its
//! surface is, handed the keys that were typed at it, and answers with a
//! [`neotmux_proto::Frame`] — what to show, not how to show it. Turning that into a screen,
//! reading a terminal, and catching the signals a process gets belong to the binary.

pub mod clip;
pub mod decode;
pub mod input;
pub mod link;
pub mod logging;
pub mod notify;
pub mod plugins;
pub mod runner;
pub mod session;
pub mod sock;

// A frame, a key, the geometry and the events all live in `neotmux-proto`: they are what
// the wire carries.

mod paneinfo;
mod procinfo;
mod shells;

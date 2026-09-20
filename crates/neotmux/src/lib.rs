//! Both halves of neotmux, with no terminal in either.
//!
//! [`session`] is the panes, the windows and the shells behind them; [`plugins`] is the Lua
//! layer and the notifications it raises. M3 puts them in two processes — the session where
//! the jobs are, the plugins where the screen is — so what is left here is what they say to
//! each other: a tick out of one, its reply out of the other, and the frame that comes of
//! it. [`link`] and [`sock`] are the wire between them and where it lives.
//!
//! Nothing here knows what it is being looked at through. A session is told how big its
//! surface is, handed the keys that were typed at it, and answers with a
//! [`neotmux_proto::Frame`] — what to show, not how to show it. Turning that into a screen,
//! reading a terminal, and catching the signals a process gets belong to the binary.

pub mod decode;
pub mod input;
pub mod link;
pub mod logging;
pub mod notify;
pub mod panes;
pub mod plugins;
pub mod reads;
pub mod session;
pub mod sock;

// A frame, a key, the geometry and the events all live in `neotmux-proto`: they are what
// the wire carries.

mod paneinfo;
mod procinfo;
mod shells;
mod windows;

//! A neotmux session: the panes, the windows, the shells behind them, and the Lua layer
//! wired to all three.
//!
//! Nothing here knows what it is being looked at through. A session is told how big its
//! surface is, handed the keys that were typed at it, and answers with a [`frame::Frame`] —
//! what to show, not how to show it. Turning that into a screen, reading a terminal, and
//! catching the signals a process gets belong to whoever is driving the session.

pub mod decode;
pub mod input;
pub mod logging;
pub mod notify;
pub mod panes;
pub mod session;

// Both moved into `neotmux-proto`: a frame and a key are what the wire carries.
pub use neotmux_proto::{frame, keys};

mod paneinfo;
mod procinfo;
mod shells;
mod windows;

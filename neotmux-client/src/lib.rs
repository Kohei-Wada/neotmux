//! The half that drives a terminal, and owns no session.
//!
//! A client is asked for one of four things, and they are this half's front door:
//! [`attach()`], [`ls`], [`kill`] and [`command`]. The last three say a sentence over the
//! socket and print the answer; [`attach()`] is the one that takes a terminal, and what is
//! in `client.rs` is the loop it then runs — it reads the terminal through `tty`, turns what
//! it read into keys with a [`Decoder`], sends them over a `Link` that `start` found or started
//! a server for, and paints the frame that comes back with `draw` and `overlay`. It holds
//! no session state — what it shows is what it was last sent.

mod ask;
mod attach;
mod client;
mod decoder;
pub(crate) mod draw;
mod osc52;
pub(crate) mod overlay;
pub(crate) mod start;

pub use ask::{command, kill, ls};
pub use attach::attach;
pub(crate) use client::*;
pub use decoder::*;

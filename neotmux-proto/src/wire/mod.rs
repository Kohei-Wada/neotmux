//! The messages two neotmux processes send each other.
//!
//! The split puts the session — the panes, the layout, the terminals — and the Lua layer that
//! talks about them in a server, and the screen in a client. So the client sends what only it
//! knows (its size, the keys it read) and is sent the picture; the frame loop is a function
//! call inside the server.
//!
//! [`Tick`] and [`Tock`] are the vocabulary of the frame as a conversation, from when the Lua
//! layer was the client's. Nothing puts them on the wire any more — they are what a session
//! hands its plugins and gets back, in one process — and they are here rather than beside
//! that call because [`Step`], which a tick carries, still rides on a [`Done`] and so is
//! still a thing the codec has to know.
//!
//! Everything here is data. The `encode_*` and `decode_*` functions (`codec.rs`) turn it into bytes; who reads and writes those
//! bytes is the socket's owner's business.

mod attach;
mod codec;
mod done;
mod end;
mod owned_done;
mod owned_to_client;
mod reader;
mod session_op;
mod step;
mod tags;
mod tick;
mod to_client;
mod to_server;
mod tock;
mod wire_error;
mod writer;

pub use attach::*;
pub use codec::*;
pub use done::*;
pub use end::*;
pub use owned_done::*;
pub use owned_to_client::*;
pub(crate) use reader::*;
pub use session_op::*;
pub use step::*;
pub use tick::*;
pub use to_client::*;
pub use to_server::*;
pub use tock::*;
pub use wire_error::*;
pub(crate) use writer::*;

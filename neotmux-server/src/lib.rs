//! The process that holds the session.
//!
//! It has no terminal: it owns the panes, their children, the tree they are laid out in and
//! the terminal state behind each of them, and it never looks at it. It owns the Lua layer
//! too, one for the session and not one per client, so a frame is no longer a conversation:
//! the session says what happened, the plugins answer, the session applies that, and every
//! attached client is sent the picture that came of it.
//!
//! Its life is the session's: when a client goes away the session is kept until its last
//! shell exits, still reading its children, so a disconnect is not a crash. A client that
//! arrives afterwards is shown that same session — and so is one that arrives while any
//! one else is looking, the one the old code refused with a reason, and the two
//! difference being the one the other is not in. Which clients get to look at a session
//! is as far as this file goes; that a session sits waiting at all is the rest of it.
//!
//! This is the half's front door and the rest of it hangs below: a [`Session`] and its `Shells`
//! are the panes and the children behind them, [`Plugins`] and `Notifications` the Lua layer and
//! what it raises, [`Processes`] a job in a real process, `Clip` a yank with nowhere of its own
//! to go, [`input`] the bytes a key is typed as, [`command`] what the host answers without the
//! Lua layer, and `PaneInfoCache` what `/proc` last said about a pane's child. `sessions.rs` holds
//! every session and moves clients between them, and `doorstep.rs` is where a new connection
//! waits to say what it wants.
//!
//! [`run`] is the door: `--server` enters here, and the loop in `server.rs` is what it enters.

pub mod command;
pub mod input;

mod answer;
mod apply;
mod clip;
mod doorstep;
mod drained;
mod drawn;
mod notifications;
mod pane_info_cache;
mod pane_row;
mod pass;
mod plugins;
mod processes;
mod server;
mod server_op;
mod session;
mod sessions;
mod setup;
mod shells;

pub use answer::*;
pub use apply::*;
pub(crate) use clip::*;
pub(crate) use drained::*;
pub(crate) use drawn::*;
pub(crate) use notifications::*;
pub(crate) use pane_info_cache::*;
pub use pane_row::*;
pub use pass::*;
pub use plugins::*;
pub use processes::*;
pub use server::*;
pub use server_op::*;
pub use session::*;
pub(crate) use sessions::*;
pub use setup::*;
pub(crate) use shells::*;

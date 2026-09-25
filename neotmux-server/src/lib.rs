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
//! This is the half's front door and the rest of it hangs below: [`session`] and `shells`
//! are the panes and the children behind them, [`plugins`] and `notify` the Lua layer and
//! what it raises, [`runner`] a job in a real process, `clip` a yank with nowhere of its
//! own to go, [`input`] the bytes a key is typed as, [`command`] what the host answers
//! without the Lua layer, and `paneinfo` what `/proc` last said about a pane's child.
//!
//! [`run`] is the door: `--server` enters here, and the loop in `server.rs` is what it enters.

pub(crate) mod clip;
pub mod command;
pub mod input;
pub(crate) mod notify;
pub(crate) mod paneinfo;
pub mod plugins;
pub mod runner;
mod server;
pub mod session;
pub(crate) mod shells;

pub use server::*;

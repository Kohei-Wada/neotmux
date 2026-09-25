//! Where panes sit: the pane tree ([`Tree`]), the one function that turns it into rectangles
//! ([`assign()`]), a window's layout over a surface ([`PaneLayout`]) and the set of windows
//! ([`Windows`]). It knows panes only by [`PaneId`](neotmux_proto::PaneId) — no PTY, no Lua.
//!
//! Geometry lives in [`assign()`] and nowhere else, so that a Lua layout callback can later stand
//! in for that one function instead of replacing the tree (issue #27). For the same reason this
//! crate never depends on `neotmux-lua`: the seam belongs to the server, which sees both.

mod assign;
mod closed;
mod pane_ids;
mod pane_layout;
mod resize_refused;
mod shown;
mod split_refused;
mod surface;
mod tree;
mod windows;

pub use assign::*;
pub use closed::*;
pub use pane_ids::*;
pub use pane_layout::*;
pub use resize_refused::*;
pub use shown::*;
pub use split_refused::*;
pub use surface::*;
pub use tree::*;
pub use windows::*;

#[cfg(test)]
mod fixtures;

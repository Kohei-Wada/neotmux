pub mod draw;
pub mod event;
pub mod frame;
pub mod geom;
pub mod geometry;
pub mod grid;
pub mod ids;
pub mod keys;
pub mod level;
pub mod msg;
pub mod paneinfo;
pub mod panerects;
pub mod read;
pub mod reserve;
pub mod width;
pub mod wire;

pub use draw::{DrawCmd, Epoch, Rect};
pub use event::Event;
pub use frame::{Frame, Notice, OwnedFrame, OwnedPaneFrame, PaneFrame};
pub use geom::{Dir, Size, SplitDir};
pub use geometry::{Geometry, Window};
pub use grid::{Color, GridSnapshot, SnapCell};
pub use ids::{PaneId, SessionId, WindowId};
pub use keys::{Key, KeyCode, Modifiers};
pub use level::Level;
pub use msg::{OwnedToClient, ToClient, ToServer};
pub use paneinfo::{LineMode, PaneInfo, PaneTty};
pub use panerects::PaneRects;
pub use read::{Extent, Lines, PaneRead, Trailing, Want};
pub use reserve::{Axis, Edge, Reserved};
pub use wire::WireError;

/// Bumped whenever any type here changes shape; checked before the first frame decodes so a
/// version mismatch fails outright instead of trying to survive one.
///
/// Every bump records what changed and whether a renderer needed it. Three bumps no renderer
/// needed is the spec's trigger (§2.1) for giving `DrawCmd` its own mirror in the codec, and
/// that count is only checkable if it is written down here.
///
/// - 1: initial version.
/// - 2: the frame stopped being the only thing on the wire. A tick, its reply, the ops, the
///   events, the geometry and the pane reads all cross now, because the Lua layer runs in
///   the client (§11, M3). A renderer needed every one of them.
/// - 3: `SessionOp::Detach` and `End::Detach`, so a client can leave without ending the
///   session. A renderer needed both.
pub const PROTOCOL_VERSION: u32 = 11;

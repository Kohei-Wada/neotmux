pub mod draw;
pub mod frame;
pub mod geom;
pub mod grid;
pub mod ids;
pub mod keys;
pub mod level;
pub mod paneinfo;
pub mod panerects;
pub mod reserve;

pub use draw::{DrawCmd, Epoch, Rect};
pub use frame::{Frame, Notice, OwnedFrame, OwnedPaneFrame, PaneFrame};
pub use geom::{Dir, Size, SplitDir};
pub use grid::{Color, GridSnapshot, SnapCell};
pub use ids::{PaneId, WindowId};
pub use keys::{Key, KeyCode, Modifiers};
pub use level::Level;
pub use paneinfo::PaneInfo;
pub use panerects::PaneRects;
pub use reserve::{Axis, Edge, Reserved};

/// Bumped whenever any type here changes shape; checked before the first frame decodes so a
/// version mismatch fails outright instead of trying to survive one.
///
/// Every bump records what changed and whether a renderer needed it. Three bumps no renderer
/// needed is the spec's trigger (§2.1) for giving `DrawCmd` its own mirror in the codec, and
/// that count is only checkable if it is written down here.
///
/// - 1: initial version.
pub const PROTOCOL_VERSION: u32 = 1;

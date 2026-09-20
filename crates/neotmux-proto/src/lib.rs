pub mod draw;
pub mod geom;
pub mod grid;
pub mod ids;
pub mod level;
pub mod paneinfo;
pub mod panerects;
pub mod reserve;

pub use draw::{DrawCmd, Epoch, Rect};
pub use geom::{Dir, Size, SplitDir};
pub use grid::{Color, GridSnapshot, SnapCell};
pub use ids::{PaneId, WindowId};
pub use level::Level;
pub use paneinfo::PaneInfo;
pub use panerects::PaneRects;
pub use reserve::{Axis, Edge, Reserved};

/// Bumped whenever any type here changes shape; checked before the first frame decodes so a
/// version mismatch fails outright instead of trying to survive one.
///
/// - 1: initial version.
pub const PROTOCOL_VERSION: u32 = 1;

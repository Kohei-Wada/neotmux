use std::collections::HashMap;

use crate::{PaneId, Rect};

/// Where each pane currently sits, in surface-local coordinates (the same
/// space `LuaSubsystem::begin_frame` was given).
pub type PaneRects = HashMap<PaneId, Rect>;

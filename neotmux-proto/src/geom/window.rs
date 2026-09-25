use crate::{PaneId, WindowId};

/// A window as Lua can ask about it: its id, and the panes it holds in layout order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    /// Left to right, top to bottom. A hidden window's order is the one it was last
    /// assigned, which is the arrangement it will be shown in.
    pub panes: Vec<PaneId>,
    /// What somebody called it, or `None` for one nobody has named. A number is what a window
    /// has until then, and a number is the one thing about it that changes when another window
    /// closes.
    pub name: Option<String>,
}

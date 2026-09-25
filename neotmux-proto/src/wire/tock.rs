use crate::wire::SessionOp;
use crate::{DrawCmd, Reserved};

/// The client's half of a frame: what the tick asked for, and what it drew.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tock {
    /// Each op with the name of the source that raised it, because a refusal is logged
    /// against whoever asked and the source table is on the client.
    pub ops: Vec<(String, SessionOp)>,
    pub overlay: Vec<DrawCmd>,
    pub reserved: Reserved,
    pub gutter: u16,
    pub wants_pane_output: bool,
}

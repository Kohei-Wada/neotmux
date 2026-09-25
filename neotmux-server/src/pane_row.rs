use neotmux_proto::{PaneId, WindowId};

/// One pane, as a command that lists them says it. The size is not here: it is the
/// terminal's, which is the whole server's rather than one session's.
pub struct PaneRow {
    pub pane: PaneId,
    pub window: WindowId,
    pub command: Option<String>,
    pub cwd: Option<String>,
}

use neotmux_proto::PaneId;

/// What one pass over every pane's output found.
pub struct Drained {
    /// Some pane produced output, so the screen needs redrawing.
    pub output: bool,
    /// The panes whose child reached EOF and are to be closed, in pane order.
    pub closed: Vec<PaneId>,
}

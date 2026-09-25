use neotmux_proto::WindowId;

/// What closing a pane did.
pub struct Closed {
    pub window: WindowId,
    /// The pane was the last of its window, so the window went with it.
    pub window_closed: bool,
    /// The window now being shown, when closing changed which one that is.
    pub switched_to: Option<WindowId>,
}

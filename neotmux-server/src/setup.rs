use neotmux_proto::{Reserved, Size, WindowId};

/// What a session is started with, or refitted to.
///
/// The size is the attaching client's, because the terminal is. The other five are the Lua
/// layer's — the window id counter, the bands the plugins reserved, the gutter between panes,
/// the columns every pane gives up, and whether anything is watching pane output — and this
/// file has no way to reach it.
pub struct Setup {
    pub size: Size,
    pub first_window: WindowId,
    pub reserved: Reserved,
    pub gutter: u16,
    pub pane_band: u16,
    pub wants_pane_output: bool,
    pub wants_pane_changed: bool,
    pub bands: u64,
}

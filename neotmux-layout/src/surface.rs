use neotmux_proto::{Reserved, Size};

/// Everything a pane tree has to be laid out against: the whole terminal, the bands taken off
/// its edges, and the cells left between panes. One value because every window is assigned
/// onto the same one, and a window that was hidden while it changed has to be told all
/// four at once.
///
/// `minimum` is only read when a window is created (`push`): a later `Surface` carrying a
/// different `minimum` to `resurface`/`show`/`close_pane` does not move it, since the layout
/// keeps the value it was constructed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub size: Size,
    pub reserved: Reserved,
    pub gutter: u16,
    pub minimum: Size,
}

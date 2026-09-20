use neotmux_proto::{DrawCmd, GridSnapshot, Level, PaneId, PaneRects, Rect as PRect, Size};

/// One pane's place in a frame, and the copy of its screen to put there.
pub struct PaneFrame<'a> {
    pub pane: PaneId,
    pub rect: PRect,
    pub grid: &'a GridSnapshot,
}

/// A notification as a frame carries it: what it says, not where it goes or what colour it
/// is drawn in.
pub struct Notice {
    pub level: Level,
    pub tag: String,
    pub text: String,
}

/// Everything one frame shows, and nothing about how it is shown. The session produces one;
/// whatever is looking at the session turns it into cells. Nothing here is a ratatui type,
/// a terminal, or a handle to one — which is what lets a frame be produced for a screen
/// that is somewhere else.
pub struct Frame<'a> {
    /// The surface the panes were laid out against: the whole screen, reserved bands
    /// included.
    pub size: Size,
    /// The panes big enough to be shown, in layout order, each with the copy of its screen.
    pub panes: Vec<PaneFrame<'a>>,
    /// Where every pane of the shown window sits, the ones too small to draw included: a
    /// draw command bound to a pane is clipped to that pane's rect either way.
    pub rects: &'a PaneRects,
    /// The pane the one cursor belongs in, which is the focused one. It is a pane rather
    /// than a position because the position is the pane's own, and the pane may not be
    /// among the ones being shown.
    pub cursor: Option<PaneId>,
    /// What the sources drew this frame, in surface coordinates or in a pane's, as each
    /// command says.
    pub overlay: &'a [DrawCmd],
    /// The notifications still live this frame, newest first.
    pub notices: Vec<Notice>,
}

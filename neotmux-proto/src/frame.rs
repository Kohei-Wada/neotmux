use crate::{DrawCmd, GridSnapshot, Level, PaneId, PaneRects, Rect, Size};

/// One pane's place in a frame, and the copy of its screen to put there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneFrame<'a> {
    pub pane: PaneId,
    pub rect: Rect,
    pub grid: &'a GridSnapshot,
}

/// A notification as the host's surface holds it: what it says, not where it goes or what
/// colour it is drawn in. It reaches a plugin through `neotmux.notifications.list()`, and
/// whoever draws it decides the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub tag: String,
    pub text: String,
}

/// Everything one frame shows, and nothing about how it is shown. The session produces one;
/// whatever is looking at the session turns it into cells. Nothing here is a ratatui type,
/// a terminal, or a handle to one — which is what lets a frame be produced for a screen
/// that is somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// A [`PaneFrame`] that owns its grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPaneFrame {
    pub pane: PaneId,
    pub rect: Rect,
    pub grid: GridSnapshot,
}

/// What a decoded frame is held in. [`Frame`] borrows so that encoding a grid can write
/// straight through the borrow; the far end has nothing to borrow from until it has decoded,
/// so it decodes into this and lends a [`Frame`] back out with [`OwnedFrame::frame`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnedFrame {
    pub size: Size,
    pub panes: Vec<OwnedPaneFrame>,
    pub rects: PaneRects,
    pub cursor: Option<PaneId>,
    pub overlay: Vec<DrawCmd>,
}

impl OwnedFrame {
    pub fn frame(&self) -> Frame<'_> {
        Frame {
            size: self.size,
            panes: self
                .panes
                .iter()
                .map(|p| PaneFrame {
                    pane: p.pane,
                    rect: p.rect,
                    grid: &p.grid,
                })
                .collect(),
            rects: &self.rects,
            cursor: self.cursor,
            overlay: &self.overlay,
        }
    }
}

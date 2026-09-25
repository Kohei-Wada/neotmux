use crate::screen::Frame;
use crate::screen::OwnedPaneFrame;
use crate::screen::PaneFrame;
use crate::{DrawCmd, PaneId, PaneRects, Size};

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

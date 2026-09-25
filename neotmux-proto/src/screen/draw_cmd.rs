use crate::screen::Rect;
use crate::{Color, PaneId};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrawCmd {
    Text {
        pane: Option<PaneId>,
        x: u16,
        y: u16,
        text: String,
        fg: Color,
        bg: Color,
    },
    Rect {
        pane: Option<PaneId>,
        rect: Rect,
        fg: Color,
        bg: Color,
    },
    /// Recolours the cells of a rectangle and writes no glyph: whatever is under it stays
    /// legible. `None` for a channel leaves that channel as it was found.
    Tint {
        pane: Option<PaneId>,
        rect: Rect,
        fg: Option<Color>,
        bg: Option<Color>,
    },
    Line {
        pane: Option<PaneId>,
        from: (u16, u16),
        to: (u16, u16),
        /// One cell's worth: the renderer writes it into every cell of the line.
        glyph: char,
        fg: Color,
    },
}

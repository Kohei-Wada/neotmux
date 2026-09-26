use super::tags::*;
use crate::screen::{Frame, PaneFrame};
use crate::wire::End;
use crate::{Color, DrawCmd, GridSnapshot, PaneId, PaneRects, Rect, Size, SnapCell};
use crate::{Key, KeyCode};

pub(crate) struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }

    /// Counts are `u32` on the wire: a frame with more than four billion of anything is not
    /// a frame this program produced.
    pub(crate) fn len(&mut self, v: usize) {
        self.u32(v as u32);
    }

    pub(crate) fn str(&mut self, v: &str) {
        self.len(v.len());
        self.bytes.extend_from_slice(v.as_bytes());
    }

    pub(crate) fn size(&mut self, v: Size) {
        self.u16(v.rows);
        self.u16(v.cols);
    }

    fn rect(&mut self, v: Rect) {
        self.u16(v.x);
        self.u16(v.y);
        self.u16(v.w);
        self.u16(v.h);
    }

    fn color(&mut self, v: Color) {
        match v {
            Color::Default => self.u8(COLOR_DEFAULT),
            Color::Idx(i) => {
                self.u8(COLOR_IDX);
                self.u8(i);
            }
            Color::Rgb(r, g, b) => {
                self.u8(COLOR_RGB);
                self.u8(r);
                self.u8(g);
                self.u8(b);
            }
        }
    }

    fn opt_color(&mut self, v: Option<Color>) {
        match v {
            None => self.bool(false),
            Some(c) => {
                self.bool(true);
                self.color(c);
            }
        }
    }

    fn opt_pane(&mut self, v: Option<PaneId>) {
        match v {
            None => self.bool(false),
            Some(p) => {
                self.bool(true);
                self.u32(p.get());
            }
        }
    }

    fn cell(&mut self, v: &SnapCell) {
        self.str(&v.text);
        self.color(v.fg);
        self.color(v.bg);
        let mut flags = 0u8;
        for (bit, set) in [
            (CELL_BOLD, v.bold),
            (CELL_ITALIC, v.italic),
            (CELL_UNDERLINE, v.underline),
            (CELL_INVERSE, v.inverse),
            (CELL_WIDE, v.wide),
            (CELL_WIDE_CONTINUATION, v.wide_continuation),
            (CELL_DIM, v.dim),
        ] {
            if set {
                flags |= bit;
            }
        }
        self.u8(flags);
    }

    fn grid(&mut self, v: &GridSnapshot) {
        self.size(v.size);
        self.u16(v.cursor.0);
        self.u16(v.cursor.1);
        self.bool(v.hide_cursor);
        self.len(v.cells.len());
        for cell in &v.cells {
            self.cell(cell);
        }
    }

    fn pane_frame(&mut self, v: &PaneFrame<'_>) {
        self.u32(v.pane.get());
        self.rect(v.rect);
        self.grid(v.grid);
    }

    fn pane_rects(&mut self, v: &PaneRects) {
        // Sorted so that one frame is one sequence of bytes, whatever order the map iterates.
        let mut entries: Vec<_> = v.iter().collect();
        entries.sort_by_key(|(pane, _)| **pane);
        self.len(entries.len());
        for (pane, rect) in entries {
            self.u32(pane.get());
            self.rect(*rect);
        }
    }

    fn draw_cmd(&mut self, v: &DrawCmd) {
        match v {
            DrawCmd::Text {
                pane,
                x,
                y,
                text,
                fg,
                bg,
                bold,
            } => {
                self.u8(CMD_TEXT);
                self.opt_pane(*pane);
                self.u16(*x);
                self.u16(*y);
                self.str(text);
                self.color(*fg);
                self.color(*bg);
                self.u8(u8::from(*bold));
            }
            DrawCmd::Rect { pane, rect, fg, bg } => {
                self.u8(CMD_RECT);
                self.opt_pane(*pane);
                self.rect(*rect);
                self.color(*fg);
                self.color(*bg);
            }
            DrawCmd::Tint { pane, rect, fg, bg } => {
                self.u8(CMD_TINT);
                self.opt_pane(*pane);
                self.rect(*rect);
                self.opt_color(*fg);
                self.opt_color(*bg);
            }
            DrawCmd::Line {
                pane,
                from,
                to,
                glyph,
                fg,
            } => {
                self.u8(CMD_LINE);
                self.opt_pane(*pane);
                self.u16(from.0);
                self.u16(from.1);
                self.u16(to.0);
                self.u16(to.1);
                self.u32(u32::from(*glyph));
                self.color(*fg);
            }
        }
    }

    pub(crate) fn key(&mut self, v: Key) {
        match v.code {
            KeyCode::Char(c) => {
                self.u8(KEY_CHAR);
                self.u32(u32::from(c));
            }
            KeyCode::F(n) => {
                self.u8(KEY_F);
                self.u8(n);
            }
            KeyCode::Enter => self.u8(KEY_ENTER),
            KeyCode::Tab => self.u8(KEY_TAB),
            KeyCode::BackTab => self.u8(KEY_BACKTAB),
            KeyCode::Backspace => self.u8(KEY_BACKSPACE),
            KeyCode::Esc => self.u8(KEY_ESC),
            KeyCode::Up => self.u8(KEY_UP),
            KeyCode::Down => self.u8(KEY_DOWN),
            KeyCode::Left => self.u8(KEY_LEFT),
            KeyCode::Right => self.u8(KEY_RIGHT),
            KeyCode::Home => self.u8(KEY_HOME),
            KeyCode::End => self.u8(KEY_END),
            KeyCode::PageUp => self.u8(KEY_PAGE_UP),
            KeyCode::PageDown => self.u8(KEY_PAGE_DOWN),
            KeyCode::Insert => self.u8(KEY_INSERT),
            KeyCode::Delete => self.u8(KEY_DELETE),
        }
        self.u8(v.mods.param());
    }
}

impl Writer {
    pub(crate) fn done(&mut self, v: &crate::wire::Done<'_>) {
        match v.frame {
            None => self.bool(false),
            Some(frame) => {
                self.bool(true);
                self.frame(frame);
            }
        }
        match v.end {
            None => self.bool(false),
            Some(end) => {
                self.bool(true);
                self.u8(match end {
                    End::Quit => END_QUIT,
                    End::Eof => END_EOF,
                    End::Detach => END_DETACH,
                });
            }
        }
    }

    pub(crate) fn frame(&mut self, v: &Frame<'_>) {
        self.size(v.size);
        self.len(v.panes.len());
        for pane in &v.panes {
            self.pane_frame(pane);
        }
        self.pane_rects(v.rects);
        self.opt_pane(v.cursor);
        self.len(v.overlay.len());
        for cmd in v.overlay {
            self.draw_cmd(cmd);
        }
    }
}

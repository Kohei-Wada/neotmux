use super::tags::*;
use crate::screen::{Frame, PaneFrame};
use crate::wire::{End, SessionOp, Step, Tick, Tock};
use crate::{
    Color, Dir, DrawCmd, Epoch, Event, Geometry, GridSnapshot, PaneId, PaneInfo, PaneRects, Rect,
    Reserved, Size, SnapCell, SplitDir, WindowId,
};
use crate::{Key, KeyCode};
use std::collections::BTreeMap;

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
    fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn opt_str(&mut self, v: Option<&str>) {
        match v {
            None => self.bool(false),
            Some(s) => {
                self.bool(true);
                self.str(s);
            }
        }
    }

    fn opt_window(&mut self, v: Option<WindowId>) {
        match v {
            None => self.bool(false),
            Some(w) => {
                self.bool(true);
                self.u32(w.get());
            }
        }
    }

    fn reserved(&mut self, v: Reserved) {
        self.u16(v.top);
        self.u16(v.bottom);
        self.u16(v.left);
        self.u16(v.right);
    }

    fn split_dir(&mut self, v: SplitDir) {
        self.u8(match v {
            SplitDir::Horizontal => SPLIT_HORIZONTAL,
            SplitDir::Vertical => SPLIT_VERTICAL,
        });
    }

    fn dir(&mut self, v: Dir) {
        self.u8(match v {
            Dir::Left => DIR_LEFT,
            Dir::Right => DIR_RIGHT,
            Dir::Up => DIR_UP,
            Dir::Down => DIR_DOWN,
        });
    }

    fn epoch(&mut self, v: &Epoch) {
        self.size(v.size());
        self.len(v.panes().len());
        for pane in v.panes() {
            self.u32(pane.get());
        }
        self.opt_pane(v.focus());
    }

    fn geometry(&mut self, v: &Geometry) {
        self.rect(v.area);
        self.len(v.panes.len());
        for (pane, rect) in &v.panes {
            self.u32(pane.get());
            self.rect(*rect);
        }
        self.opt_pane(v.focus);
        self.len(v.windows.len());
        for window in &v.windows {
            self.u32(window.id.get());
            self.len(window.panes.len());
            for pane in &window.panes {
                self.u32(pane.get());
            }
            self.opt_str(window.name.as_deref());
        }
        self.opt_window(v.window);
    }

    fn pane_info(&mut self, v: &BTreeMap<PaneId, PaneInfo>) {
        self.len(v.len());
        for (pane, info) in v {
            self.u32(pane.get());
            self.opt_str(info.command.as_deref());
            self.opt_str(info.cwd.as_deref());
        }
    }

    fn event(&mut self, v: &Event) {
        match v {
            Event::PaneOutput { pane, bytes } => {
                self.u8(EVENT_PANE_OUTPUT);
                self.u32(pane.get());
                self.len(bytes.len());
                self.bytes.extend_from_slice(bytes);
            }
            Event::Resized { size } => {
                self.u8(EVENT_RESIZED);
                self.size(*size);
            }
            Event::PaneOpened { pane } => {
                self.u8(EVENT_PANE_OPENED);
                self.u32(pane.get());
            }
            Event::PaneClosed { pane } => {
                self.u8(EVENT_PANE_CLOSED);
                self.u32(pane.get());
            }
            Event::FocusChanged { pane } => {
                self.u8(EVENT_FOCUS_CHANGED);
                self.u32(pane.get());
            }
            Event::WindowOpened { window } => {
                self.u8(EVENT_WINDOW_OPENED);
                self.u32(window.get());
            }
            Event::WindowClosed { window } => {
                self.u8(EVENT_WINDOW_CLOSED);
                self.u32(window.get());
            }
            Event::WindowChanged { window } => {
                self.u8(EVENT_WINDOW_CHANGED);
                self.u32(window.get());
            }
        }
    }

    fn step(&mut self, v: &Step) {
        match v {
            Step::Geometry(geometry) => {
                self.u8(STEP_GEOMETRY);
                self.geometry(geometry);
            }
            Step::Bands { surface, applied } => {
                self.u8(STEP_BANDS);
                self.size(*surface);
                self.reserved(*applied);
            }
            Step::Event(event) => {
                self.u8(STEP_EVENT);
                self.event(event);
            }
        }
    }

    pub(crate) fn tick(&mut self, v: &Tick) {
        self.len(v.steps.len());
        for step in &v.steps {
            self.step(step);
        }
        self.epoch(&v.epoch);
        self.geometry(&v.geometry);
        match &v.pane_info {
            None => self.bool(false),
            Some(info) => {
                self.bool(true);
                self.pane_info(info);
            }
        }
        self.len(v.pane_tty.len());
        for (pane, tty) in &v.pane_tty {
            self.u32(pane.get());
            self.u16(tty.cursor.0);
            self.u16(tty.cursor.1);
            self.bool(tty.cursor_visible);
            self.bool(tty.alternate);
            self.bool(tty.application_cursor);
            match &tty.line {
                None => self.bool(false),
                Some(line) => {
                    self.bool(true);
                    self.bool(line.canonical);
                    self.bool(line.echo);
                }
            }
        }
        self.len(v.revisions.len());
        for (pane, revision) in &v.revisions {
            self.u32(pane.get());
            self.u64(*revision);
        }
    }

    fn op(&mut self, v: &SessionOp) {
        match v {
            SessionOp::Quit => self.u8(OP_QUIT),
            SessionOp::Detach => self.u8(OP_DETACH),
            SessionOp::PaneSplit { dir } => {
                self.u8(OP_PANE_SPLIT);
                self.split_dir(*dir);
            }
            SessionOp::PaneSend { pane, text } => {
                self.u8(OP_PANE_SEND);
                self.u32(pane.get());
                self.str(text);
            }
            SessionOp::PaneFocus { pane } => {
                self.u8(OP_PANE_FOCUS);
                self.u32(pane.get());
            }
            SessionOp::PaneResize { dir, cells } => {
                self.u8(OP_PANE_RESIZE);
                self.dir(*dir);
                self.u16(*cells);
            }
            SessionOp::WindowCreate { window } => {
                self.u8(OP_WINDOW_CREATE);
                self.u32(window.get());
            }
            SessionOp::WindowFocus { window } => {
                self.u8(OP_WINDOW_FOCUS);
                self.u32(window.get());
            }
            SessionOp::WindowRename { window, name } => {
                self.u8(OP_WINDOW_RENAME);
                self.u32(window.get());
                self.opt_str(name.as_deref());
            }
        }
    }

    pub(crate) fn tock(&mut self, v: &Tock) {
        self.len(v.ops.len());
        for (tag, op) in &v.ops {
            self.str(tag);
            self.op(op);
        }
        self.len(v.overlay.len());
        for cmd in &v.overlay {
            self.draw_cmd(cmd);
        }
        self.reserved(v.reserved);
        self.u16(v.gutter);
        self.bool(v.wants_pane_output);
    }

    pub(crate) fn done(&mut self, v: &crate::wire::Done<'_>) {
        self.len(v.steps.len());
        for step in &v.steps {
            self.step(step);
        }
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

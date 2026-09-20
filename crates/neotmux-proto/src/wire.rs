//! The wire: the two messages that cross a connection, written to bytes and read back.
//!
//! Server to client is a frame, client to server is a key, and before either of them the
//! connection opens with [`PROTOCOL_VERSION`]. A peer that sends a different number is
//! refused by [`decode_hello`] naming both, which is the whole of the compatibility story
//! (§2.1): every speaker of this protocol is built from the same commit as its peer, so skew
//! is a case to detect rather than one to negotiate. Nothing here skips an unknown
//! discriminant — an unknown one is a version mismatch that the handshake should already
//! have caught, and it is reported as malformed.
//!
//! The encoding is little-endian, length-prefixed, and has no framing of its own: a message
//! is a byte slice, and whoever owns the socket decides where one ends.

use std::fmt;

use crate::frame::{Frame, Notice, OwnedFrame, OwnedPaneFrame, PaneFrame};
use crate::keys::{Key, KeyCode, Modifiers};
use crate::{
    Color, DrawCmd, GridSnapshot, Level, PROTOCOL_VERSION, PaneId, PaneRects, Rect, Size, SnapCell,
};

/// The length of the handshake, which is the one message with a fixed size.
pub const HELLO_LEN: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The peer was built against a different protocol. Distinct from every other error
    /// here, and carrying both numbers, because it is the one a person can act on.
    VersionMismatch { ours: u32, theirs: u32 },
    /// The bytes ran out mid-value.
    Truncated,
    /// The message decoded, and then there were bytes left over.
    Trailing,
    /// A value the format cannot mean: a bad discriminant, a bad `bool`, invalid UTF-8.
    Malformed(&'static str),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionMismatch { ours, theirs } => write!(
                f,
                "protocol version mismatch (ours {ours}, theirs {theirs})"
            ),
            Self::Truncated => f.write_str("truncated message"),
            Self::Trailing => f.write_str("trailing bytes after the message"),
            Self::Malformed(what) => write!(f, "malformed message: {what}"),
        }
    }
}

impl std::error::Error for WireError {}

/// The first thing on a connection.
pub fn encode_hello() -> [u8; HELLO_LEN] {
    PROTOCOL_VERSION.to_le_bytes()
}

/// The only check of the protocol version there is: everything else on the wire assumes it
/// has passed.
pub fn decode_hello(bytes: &[u8]) -> Result<(), WireError> {
    let mut r = Reader::new(bytes);
    let theirs = r.u32()?;
    if theirs != PROTOCOL_VERSION {
        return Err(WireError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs,
        });
    }
    r.finish()
}

pub fn encode_frame(frame: &Frame<'_>) -> Vec<u8> {
    let mut w = Writer::new();
    w.size(frame.size);
    w.len(frame.panes.len());
    for pane in &frame.panes {
        w.pane_frame(pane);
    }
    w.pane_rects(frame.rects);
    w.opt_pane(frame.cursor);
    w.len(frame.overlay.len());
    for cmd in frame.overlay {
        w.draw_cmd(cmd);
    }
    w.len(frame.notices.len());
    for notice in &frame.notices {
        w.notice(notice);
    }
    w.finish()
}

pub fn decode_frame(bytes: &[u8]) -> Result<OwnedFrame, WireError> {
    let mut r = Reader::new(bytes);
    let size = r.size()?;
    let mut panes = Vec::new();
    for _ in 0..r.len()? {
        panes.push(r.pane_frame()?);
    }
    let rects = r.pane_rects()?;
    let cursor = r.opt_pane()?;
    let mut overlay = Vec::new();
    for _ in 0..r.len()? {
        overlay.push(r.draw_cmd()?);
    }
    let mut notices = Vec::new();
    for _ in 0..r.len()? {
        notices.push(r.notice()?);
    }
    r.finish()?;
    Ok(OwnedFrame {
        size,
        panes,
        rects,
        cursor,
        overlay,
        notices,
    })
}

pub fn encode_key(key: Key) -> Vec<u8> {
    let mut w = Writer::new();
    w.key(key);
    w.finish()
}

pub fn decode_key(bytes: &[u8]) -> Result<Key, WireError> {
    let mut r = Reader::new(bytes);
    let key = r.key()?;
    r.finish()?;
    Ok(key)
}

const CMD_TEXT: u8 = 0;
const CMD_RECT: u8 = 1;
const CMD_TINT: u8 = 2;
const CMD_LINE: u8 = 3;

const COLOR_DEFAULT: u8 = 0;
const COLOR_IDX: u8 = 1;
const COLOR_RGB: u8 = 2;

const LEVEL_INFO: u8 = 0;
const LEVEL_WARN: u8 = 1;
const LEVEL_ERROR: u8 = 2;

const KEY_CHAR: u8 = 0;
const KEY_ENTER: u8 = 1;
const KEY_TAB: u8 = 2;
const KEY_BACKTAB: u8 = 3;
const KEY_BACKSPACE: u8 = 4;
const KEY_ESC: u8 = 5;
const KEY_UP: u8 = 6;
const KEY_DOWN: u8 = 7;
const KEY_LEFT: u8 = 8;
const KEY_RIGHT: u8 = 9;
const KEY_HOME: u8 = 10;
const KEY_END: u8 = 11;
const KEY_PAGE_UP: u8 = 12;
const KEY_PAGE_DOWN: u8 = 13;
const KEY_INSERT: u8 = 14;
const KEY_DELETE: u8 = 15;
const KEY_F: u8 = 16;

const CELL_BOLD: u8 = 1;
const CELL_ITALIC: u8 = 2;
const CELL_UNDERLINE: u8 = 4;
const CELL_INVERSE: u8 = 8;
const CELL_WIDE: u8 = 16;
const CELL_WIDE_CONTINUATION: u8 = 32;

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }

    /// Counts are `u32` on the wire: a frame with more than four billion of anything is not
    /// a frame this program produced.
    fn len(&mut self, v: usize) {
        self.u32(v as u32);
    }

    fn str(&mut self, v: &str) {
        self.len(v.len());
        self.bytes.extend_from_slice(v.as_bytes());
    }

    fn size(&mut self, v: Size) {
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
            } => {
                self.u8(CMD_TEXT);
                self.opt_pane(*pane);
                self.u16(*x);
                self.u16(*y);
                self.str(text);
                self.color(*fg);
                self.color(*bg);
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

    fn notice(&mut self, v: &Notice) {
        self.u8(match v.level {
            Level::Info => LEVEL_INFO,
            Level::Warn => LEVEL_WARN,
            Level::Error => LEVEL_ERROR,
        });
        self.str(&v.tag);
        self.str(&v.text);
    }

    fn key(&mut self, v: Key) {
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

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn finish(self) -> Result<(), WireError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::Trailing)
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(WireError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn bool(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::Malformed("bool")),
        }
    }

    /// A count is never used to reserve: every loop over one reads its items, and a count
    /// larger than the bytes left runs out inside the loop instead of allocating for a
    /// length nothing could have sent.
    fn len(&mut self) -> Result<usize, WireError> {
        Ok(self.u32()? as usize)
    }

    fn char(&mut self) -> Result<char, WireError> {
        char::from_u32(self.u32()?).ok_or(WireError::Malformed("char"))
    }

    fn string(&mut self) -> Result<String, WireError> {
        let n = self.len()?;
        let bytes = self.take(n)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| WireError::Malformed("utf-8"))
    }

    fn size(&mut self) -> Result<Size, WireError> {
        Ok(Size {
            rows: self.u16()?,
            cols: self.u16()?,
        })
    }

    fn rect(&mut self) -> Result<Rect, WireError> {
        Ok(Rect::new(
            self.u16()?,
            self.u16()?,
            self.u16()?,
            self.u16()?,
        ))
    }

    fn pane(&mut self) -> Result<PaneId, WireError> {
        Ok(PaneId::from(self.u32()?))
    }

    fn color(&mut self) -> Result<Color, WireError> {
        match self.u8()? {
            COLOR_DEFAULT => Ok(Color::Default),
            COLOR_IDX => Ok(Color::Idx(self.u8()?)),
            COLOR_RGB => Ok(Color::Rgb(self.u8()?, self.u8()?, self.u8()?)),
            _ => Err(WireError::Malformed("colour discriminant")),
        }
    }

    fn opt_color(&mut self) -> Result<Option<Color>, WireError> {
        if self.bool()? {
            Ok(Some(self.color()?))
        } else {
            Ok(None)
        }
    }

    fn opt_pane(&mut self) -> Result<Option<PaneId>, WireError> {
        if self.bool()? {
            Ok(Some(self.pane()?))
        } else {
            Ok(None)
        }
    }

    fn cell(&mut self) -> Result<SnapCell, WireError> {
        let text = self.string()?;
        let fg = self.color()?;
        let bg = self.color()?;
        let flags = self.u8()?;
        if flags
            & !(CELL_BOLD
                | CELL_ITALIC
                | CELL_UNDERLINE
                | CELL_INVERSE
                | CELL_WIDE
                | CELL_WIDE_CONTINUATION)
            != 0
        {
            return Err(WireError::Malformed("cell flags"));
        }
        Ok(SnapCell {
            text,
            fg,
            bg,
            bold: flags & CELL_BOLD != 0,
            italic: flags & CELL_ITALIC != 0,
            underline: flags & CELL_UNDERLINE != 0,
            inverse: flags & CELL_INVERSE != 0,
            wide: flags & CELL_WIDE != 0,
            wide_continuation: flags & CELL_WIDE_CONTINUATION != 0,
        })
    }

    fn grid(&mut self) -> Result<GridSnapshot, WireError> {
        let size = self.size()?;
        let cursor = (self.u16()?, self.u16()?);
        let hide_cursor = self.bool()?;
        let count = self.len()?;
        if count != size.area() {
            return Err(WireError::Malformed("grid cell count against its size"));
        }
        let mut cells = Vec::new();
        for _ in 0..count {
            cells.push(self.cell()?);
        }
        Ok(GridSnapshot {
            size,
            cursor,
            hide_cursor,
            cells,
        })
    }

    fn pane_frame(&mut self) -> Result<OwnedPaneFrame, WireError> {
        Ok(OwnedPaneFrame {
            pane: self.pane()?,
            rect: self.rect()?,
            grid: self.grid()?,
        })
    }

    fn pane_rects(&mut self) -> Result<PaneRects, WireError> {
        let count = self.len()?;
        let mut rects = PaneRects::new();
        for _ in 0..count {
            let pane = self.pane()?;
            let rect = self.rect()?;
            rects.insert(pane, rect);
        }
        Ok(rects)
    }

    fn draw_cmd(&mut self) -> Result<DrawCmd, WireError> {
        match self.u8()? {
            CMD_TEXT => Ok(DrawCmd::Text {
                pane: self.opt_pane()?,
                x: self.u16()?,
                y: self.u16()?,
                text: self.string()?,
                fg: self.color()?,
                bg: self.color()?,
            }),
            CMD_RECT => Ok(DrawCmd::Rect {
                pane: self.opt_pane()?,
                rect: self.rect()?,
                fg: self.color()?,
                bg: self.color()?,
            }),
            CMD_TINT => Ok(DrawCmd::Tint {
                pane: self.opt_pane()?,
                rect: self.rect()?,
                fg: self.opt_color()?,
                bg: self.opt_color()?,
            }),
            CMD_LINE => Ok(DrawCmd::Line {
                pane: self.opt_pane()?,
                from: (self.u16()?, self.u16()?),
                to: (self.u16()?, self.u16()?),
                glyph: self.char()?,
                fg: self.color()?,
            }),
            _ => Err(WireError::Malformed("draw command discriminant")),
        }
    }

    fn notice(&mut self) -> Result<Notice, WireError> {
        let level = match self.u8()? {
            LEVEL_INFO => Level::Info,
            LEVEL_WARN => Level::Warn,
            LEVEL_ERROR => Level::Error,
            _ => return Err(WireError::Malformed("level discriminant")),
        };
        Ok(Notice {
            level,
            tag: self.string()?,
            text: self.string()?,
        })
    }

    fn key(&mut self) -> Result<Key, WireError> {
        let code = match self.u8()? {
            KEY_CHAR => KeyCode::Char(self.char()?),
            KEY_F => KeyCode::F(self.u8()?),
            KEY_ENTER => KeyCode::Enter,
            KEY_TAB => KeyCode::Tab,
            KEY_BACKTAB => KeyCode::BackTab,
            KEY_BACKSPACE => KeyCode::Backspace,
            KEY_ESC => KeyCode::Esc,
            KEY_UP => KeyCode::Up,
            KEY_DOWN => KeyCode::Down,
            KEY_LEFT => KeyCode::Left,
            KEY_RIGHT => KeyCode::Right,
            KEY_HOME => KeyCode::Home,
            KEY_END => KeyCode::End,
            KEY_PAGE_UP => KeyCode::PageUp,
            KEY_PAGE_DOWN => KeyCode::PageDown,
            KEY_INSERT => KeyCode::Insert,
            KEY_DELETE => KeyCode::Delete,
            _ => return Err(WireError::Malformed("key discriminant")),
        };
        let param = self.u8()?;
        if !(1..=8).contains(&param) {
            return Err(WireError::Malformed("modifier parameter"));
        }
        Ok(Key::new(code, Modifiers::from_param(u16::from(param))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLORS: [Color; 4] = [
        Color::Default,
        Color::Idx(0),
        Color::Idx(255),
        Color::Rgb(1, 2, 3),
    ];

    fn grid_of(rows: u16, cols: u16, glyphs: &[&str]) -> GridSnapshot {
        let mut grid = GridSnapshot::new(Size { rows, cols });
        for (i, glyph) in glyphs.iter().enumerate() {
            let cell = &mut grid.cells[i];
            cell.text = (*glyph).to_owned();
            cell.fg = COLORS[i % COLORS.len()];
            cell.bold = i % 2 == 0;
            cell.underline = i % 3 == 0;
        }
        grid.cursor = (rows.saturating_sub(1), cols.saturating_sub(1));
        grid
    }

    fn frame_of(panes: Vec<OwnedPaneFrame>) -> OwnedFrame {
        let mut rects = PaneRects::new();
        for pane in &panes {
            rects.insert(pane.pane, pane.rect);
        }
        OwnedFrame {
            size: Size { rows: 24, cols: 80 },
            cursor: panes.first().map(|p| p.pane),
            panes,
            rects,
            overlay: Vec::new(),
            notices: Vec::new(),
        }
    }

    fn round_trip(frame: &OwnedFrame) {
        let bytes = encode_frame(&frame.frame());
        assert_eq!(decode_frame(&bytes).as_ref(), Ok(frame));
    }

    fn every_draw_cmd() -> Vec<DrawCmd> {
        let mut cmds = Vec::new();
        for (n, fg) in COLORS.into_iter().enumerate() {
            let pane = if n % 2 == 0 {
                Some(PaneId::from(n as u32))
            } else {
                None
            };
            cmds.push(DrawCmd::Text {
                pane,
                x: 1,
                y: 2,
                text: "日本語 and ascii".to_owned(),
                fg,
                bg: COLORS[(n + 1) % COLORS.len()],
            });
            cmds.push(DrawCmd::Rect {
                pane,
                rect: Rect::new(1, 2, 3, 4),
                fg,
                bg: COLORS[(n + 2) % COLORS.len()],
            });
            cmds.push(DrawCmd::Tint {
                pane,
                rect: Rect::new(0, 0, 1, 1),
                fg: Some(fg),
                bg: None,
            });
            cmds.push(DrawCmd::Tint {
                pane,
                rect: Rect::new(0, 0, 1, 1),
                fg: None,
                bg: Some(fg),
            });
            cmds.push(DrawCmd::Line {
                pane,
                from: (0, 0),
                to: (9, 9),
                glyph: '─',
                fg,
            });
        }
        cmds
    }

    #[test]
    fn the_handshake_round_trips() {
        assert_eq!(decode_hello(&encode_hello()), Ok(()));
    }

    #[test]
    fn a_peer_on_another_version_is_named_along_with_us() {
        let theirs = PROTOCOL_VERSION + 41;
        let err = decode_hello(&theirs.to_le_bytes()).unwrap_err();
        assert_eq!(
            err,
            WireError::VersionMismatch {
                ours: PROTOCOL_VERSION,
                theirs
            }
        );
        let said = err.to_string();
        assert!(said.contains(&PROTOCOL_VERSION.to_string()), "{said}");
        assert!(said.contains(&theirs.to_string()), "{said}");
    }

    #[test]
    fn a_short_handshake_is_truncated_rather_than_a_mismatch() {
        assert_eq!(decode_hello(&[1, 0, 0]), Err(WireError::Truncated));
    }

    #[test]
    fn every_draw_command_and_every_colour_round_trips() {
        let mut frame = frame_of(vec![]);
        frame.overlay = every_draw_cmd();
        round_trip(&frame);
    }

    #[test]
    fn a_grid_of_wide_characters_round_trips() {
        let mut grid = grid_of(2, 4, &["あ", "", "い", "", "う", "", "え", ""]);
        for (i, cell) in grid.cells.iter_mut().enumerate() {
            cell.wide = i % 2 == 0;
            cell.wide_continuation = i % 2 == 1;
        }
        round_trip(&frame_of(vec![OwnedPaneFrame {
            pane: PaneId::from(1),
            rect: Rect::new(0, 0, 4, 2),
            grid,
        }]));
    }

    #[test]
    fn a_grid_of_wrapped_lines_round_trips() {
        // A line longer than the grid is wide is stored as the cells it wrapped onto.
        let text: Vec<String> = "wrapped"
            .chars()
            .cycle()
            .take(12)
            .map(String::from)
            .collect();
        let glyphs: Vec<&str> = text.iter().map(String::as_str).collect();
        let grid = grid_of(3, 4, &glyphs);
        round_trip(&frame_of(vec![OwnedPaneFrame {
            pane: PaneId::from(7),
            rect: Rect::new(0, 0, 4, 3),
            grid,
        }]));
    }

    #[test]
    fn an_empty_grid_round_trips() {
        round_trip(&frame_of(vec![OwnedPaneFrame {
            pane: PaneId::from(1),
            rect: Rect::default(),
            grid: GridSnapshot::default(),
        }]));
        round_trip(&frame_of(vec![OwnedPaneFrame {
            pane: PaneId::from(2),
            rect: Rect::default(),
            grid: GridSnapshot::new(Size { rows: 0, cols: 40 }),
        }]));
    }

    #[test]
    fn a_frame_with_no_panes_round_trips() {
        round_trip(&frame_of(vec![]));
        round_trip(&OwnedFrame::default());
    }

    #[test]
    fn notices_round_trip_at_every_level() {
        let mut frame = frame_of(vec![]);
        frame.notices = vec![
            Notice {
                level: Level::Info,
                tag: "lua".to_owned(),
                text: "loaded".to_owned(),
            },
            Notice {
                level: Level::Warn,
                tag: String::new(),
                text: "日本語".to_owned(),
            },
            Notice {
                level: Level::Error,
                tag: "io".to_owned(),
                text: String::new(),
            },
        ];
        round_trip(&frame);
    }

    #[test]
    fn every_key_round_trips_under_every_modifier() {
        let codes = [
            KeyCode::Char('a'),
            KeyCode::Char('あ'),
            KeyCode::Char('\u{10FFFF}'),
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Backspace,
            KeyCode::Esc,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Insert,
            KeyCode::Delete,
            KeyCode::F(1),
            KeyCode::F(255),
        ];
        for code in codes {
            for param in 1..=8u16 {
                let key = Key::new(code, Modifiers::from_param(param));
                assert_eq!(decode_key(&encode_key(key)), Ok(key));
            }
        }
    }

    #[test]
    fn an_unknown_discriminant_is_malformed_rather_than_skipped() {
        assert_eq!(
            decode_key(&[200, 1]),
            Err(WireError::Malformed("key discriminant"))
        );
        let mut frame = frame_of(vec![]);
        frame.overlay = vec![DrawCmd::Rect {
            pane: None,
            rect: Rect::default(),
            fg: Color::Default,
            bg: Color::Default,
        }];
        let mut bytes = encode_frame(&frame.frame());
        // The one command, past the empty notice count: tag, pane, rect, fg, bg.
        let tag = bytes.len() - 16;
        assert_eq!(bytes[tag], CMD_RECT);
        bytes[tag] = 200;
        assert_eq!(
            decode_frame(&bytes),
            Err(WireError::Malformed("draw command discriminant"))
        );
    }

    #[test]
    fn a_truncated_payload_fails_cleanly() {
        let mut frame = frame_of(vec![OwnedPaneFrame {
            pane: PaneId::from(1),
            rect: Rect::new(0, 0, 4, 2),
            grid: grid_of(2, 4, &["a", "b", "c", "d", "e", "f", "g", "h"]),
        }]);
        frame.overlay = every_draw_cmd();
        frame.notices = vec![Notice {
            level: Level::Warn,
            tag: "t".to_owned(),
            text: "x".to_owned(),
        }];
        let bytes = encode_frame(&frame.frame());
        for n in 0..bytes.len() {
            assert!(decode_frame(&bytes[..n]).is_err(), "prefix of {n} decoded");
        }
        assert_eq!(decode_frame(&bytes), Ok(frame));
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let frame = frame_of(vec![]);
        let mut bytes = encode_frame(&frame.frame());
        bytes.push(0);
        assert_eq!(decode_frame(&bytes), Err(WireError::Trailing));
    }

    /// xorshift64*: a property test wants arbitrary frames, not a dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn bool(&mut self) -> bool {
            self.next() & 1 == 1
        }

        fn color(&mut self) -> Color {
            match self.below(3) {
                0 => Color::Default,
                1 => Color::Idx(self.next() as u8),
                _ => Color::Rgb(self.next() as u8, self.next() as u8, self.next() as u8),
            }
        }

        fn text(&mut self) -> String {
            let alphabet = ['a', 'Z', '0', ' ', 'あ', '─', '\u{1F600}', '\u{7F}'];
            (0..self.below(4))
                .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
                .collect()
        }

        fn rect(&mut self) -> Rect {
            Rect::new(
                self.next() as u16,
                self.next() as u16,
                self.next() as u16,
                self.next() as u16,
            )
        }

        fn opt_pane(&mut self) -> Option<PaneId> {
            self.bool().then(|| PaneId::from(self.next() as u32))
        }

        fn draw_cmd(&mut self) -> DrawCmd {
            match self.below(4) {
                0 => DrawCmd::Text {
                    pane: self.opt_pane(),
                    x: self.next() as u16,
                    y: self.next() as u16,
                    text: self.text(),
                    fg: self.color(),
                    bg: self.color(),
                },
                1 => DrawCmd::Rect {
                    pane: self.opt_pane(),
                    rect: self.rect(),
                    fg: self.color(),
                    bg: self.color(),
                },
                2 => DrawCmd::Tint {
                    pane: self.opt_pane(),
                    rect: self.rect(),
                    fg: self.bool().then(|| self.color()),
                    bg: self.bool().then(|| self.color()),
                },
                _ => DrawCmd::Line {
                    pane: self.opt_pane(),
                    from: (self.next() as u16, self.next() as u16),
                    to: (self.next() as u16, self.next() as u16),
                    glyph: ['|', '─', 'あ'][self.below(3) as usize],
                    fg: self.color(),
                },
            }
        }

        fn grid(&mut self) -> GridSnapshot {
            let size = Size {
                rows: self.below(5) as u16,
                cols: self.below(5) as u16,
            };
            let mut grid = GridSnapshot::new(size);
            for cell in &mut grid.cells {
                cell.text = self.text();
                cell.fg = self.color();
                cell.bg = self.color();
                cell.bold = self.bool();
                cell.italic = self.bool();
                cell.underline = self.bool();
                cell.inverse = self.bool();
                cell.wide = self.bool();
                cell.wide_continuation = self.bool();
            }
            grid.cursor = (self.next() as u16, self.next() as u16);
            grid.hide_cursor = self.bool();
            grid
        }

        fn frame(&mut self) -> OwnedFrame {
            let mut panes = Vec::new();
            for n in 0..self.below(4) {
                panes.push(OwnedPaneFrame {
                    pane: PaneId::from(n as u32),
                    rect: self.rect(),
                    grid: self.grid(),
                });
            }
            let mut rects = PaneRects::new();
            for n in 0..self.below(6) {
                rects.insert(PaneId::from(n as u32), self.rect());
            }
            let overlay = (0..self.below(5)).map(|_| self.draw_cmd()).collect();
            let notices = (0..self.below(3))
                .map(|_| Notice {
                    level: [Level::Info, Level::Warn, Level::Error][self.below(3) as usize],
                    tag: self.text(),
                    text: self.text(),
                })
                .collect();
            OwnedFrame {
                size: Size {
                    rows: self.next() as u16,
                    cols: self.next() as u16,
                },
                panes,
                rects,
                cursor: self.opt_pane(),
                overlay,
                notices,
            }
        }
    }

    #[test]
    fn arbitrary_frames_round_trip_and_arbitrary_prefixes_of_them_do_not_panic() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..500 {
            let frame = rng.frame();
            let bytes = encode_frame(&frame.frame());
            assert_eq!(decode_frame(&bytes).as_ref(), Ok(&frame));
            // Byte-for-byte stable, which is what lets a frame be compared or cached.
            assert_eq!(encode_frame(&frame.frame()), bytes);
            let cut = rng.below(bytes.len() as u64 + 1) as usize;
            if cut < bytes.len() {
                assert!(decode_frame(&bytes[..cut]).is_err());
            }
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);
        for _ in 0..500 {
            let bytes: Vec<u8> = (0..rng.below(64)).map(|_| rng.next() as u8).collect();
            let _ = decode_frame(&bytes);
            let _ = decode_key(&bytes);
            let _ = decode_hello(&bytes);
        }
    }
}

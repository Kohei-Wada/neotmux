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

use std::collections::BTreeMap;

use crate::frame::{Frame, OwnedFrame, OwnedPaneFrame, PaneFrame};
use crate::keys::{Key, KeyCode, Modifiers};
use crate::msg::{
    Attach, End, OwnedToClient, Read, Reading, SessionOp, Step, Tick, ToClient, ToServer, Tock,
};
use crate::read::{Extent, Lines, Trailing, Want};
use crate::{
    Color, Dir, DrawCmd, Edge, Epoch, Event, Geometry, GridSnapshot, PROTOCOL_VERSION, PaneId,
    PaneInfo, PaneRects, Rect, Reserved, Size, SnapCell, SplitDir, Window, WindowId,
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
    w.frame(frame);
    w.finish()
}

pub fn decode_frame(bytes: &[u8]) -> Result<OwnedFrame, WireError> {
    let mut r = Reader::new(bytes);
    let frame = r.frame()?;
    r.finish()?;
    Ok(frame)
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

const TO_SERVER_ATTACH: u8 = 0;
const TO_SERVER_KEY: u8 = 1;
const TO_SERVER_RESIZE: u8 = 2;
const TO_SERVER_READ: u8 = 3;
const TO_SERVER_TOCK: u8 = 4;
const TO_SERVER_LIST: u8 = 5;
const TO_SERVER_KILL: u8 = 6;
const TO_SERVER_COMMAND: u8 = 7;

const TO_CLIENT_TICK: u8 = 0;
const TO_CLIENT_DONE: u8 = 1;
const TO_CLIENT_READING: u8 = 2;
const TO_CLIENT_REFUSED: u8 = 3;
const TO_CLIENT_CLIP: u8 = 4;
const TO_CLIENT_BYE: u8 = 5;
const TO_CLIENT_SESSIONS: u8 = 6;
const TO_CLIENT_KILLED: u8 = 7;
const TO_CLIENT_RAN: u8 = 8;

const STEP_GEOMETRY: u8 = 0;
const STEP_BANDS: u8 = 1;
const STEP_EVENT: u8 = 2;

const EVENT_PANE_OUTPUT: u8 = 0;
const EVENT_RESIZED: u8 = 1;
const EVENT_PANE_OPENED: u8 = 2;
const EVENT_PANE_CLOSED: u8 = 3;
const EVENT_FOCUS_CHANGED: u8 = 4;
const EVENT_WINDOW_OPENED: u8 = 5;
const EVENT_WINDOW_CLOSED: u8 = 6;
const EVENT_WINDOW_CHANGED: u8 = 7;

const OP_QUIT: u8 = 0;
const OP_PANE_SPLIT: u8 = 1;
const OP_PANE_FOCUS: u8 = 2;
const OP_PANE_RESIZE: u8 = 3;
const OP_WINDOW_CREATE: u8 = 4;
const OP_WINDOW_FOCUS: u8 = 5;
const OP_DETACH: u8 = 6;
const OP_WINDOW_RENAME: u8 = 7;

const SPLIT_HORIZONTAL: u8 = 0;
const SPLIT_VERTICAL: u8 = 1;

const DIR_LEFT: u8 = 0;
const DIR_RIGHT: u8 = 1;
const DIR_UP: u8 = 2;
const DIR_DOWN: u8 = 3;

const READ_REVISION: u8 = 0;
const READ_CONTENT: u8 = 1;
const READ_HISTORY_EXTENT: u8 = 2;
const READ_HISTORY: u8 = 3;
const READ_FREEZE: u8 = 4;
const READ_UNFREEZE: u8 = 5;
const READ_IS_FROZEN: u8 = 6;

const READING_REVISION: u8 = 0;
const READING_TEXT: u8 = 1;
const READING_EXTENT: u8 = 2;
const READING_DID: u8 = 3;
const READING_FROZEN: u8 = 4;

const END_QUIT: u8 = 0;
const END_EOF: u8 = 1;
const END_DETACH: u8 = 2;

pub fn encode_to_server(msg: &ToServer) -> Vec<u8> {
    let mut w = Writer::new();
    match msg {
        ToServer::Attach(attach) => {
            w.u8(TO_SERVER_ATTACH);
            w.size(attach.size);
            w.str(&attach.session);
        }
        ToServer::Key(key) => {
            w.u8(TO_SERVER_KEY);
            w.key(*key);
        }
        ToServer::Resize(size) => {
            w.u8(TO_SERVER_RESIZE);
            w.size(*size);
        }
        ToServer::Read(read) => {
            w.u8(TO_SERVER_READ);
            w.read(*read);
        }
        ToServer::Tock(tock) => {
            w.u8(TO_SERVER_TOCK);
            w.tock(tock);
        }
        ToServer::List => w.u8(TO_SERVER_LIST),
        ToServer::Kill { session } => {
            w.u8(TO_SERVER_KILL);
            w.str(session);
        }
        ToServer::Command {
            session,
            name,
            args,
        } => {
            w.u8(TO_SERVER_COMMAND);
            w.bool(session.is_some());
            w.str(session.as_deref().unwrap_or(""));
            w.str(name);
            w.len(args.len());
            for arg in args {
                w.str(arg);
            }
        }
    }
    w.finish()
}

pub fn decode_to_server(bytes: &[u8]) -> Result<ToServer, WireError> {
    let mut r = Reader::new(bytes);
    let msg = match r.u8()? {
        TO_SERVER_ATTACH => ToServer::Attach(Attach {
            size: r.size()?,
            session: r.string()?,
        }),
        TO_SERVER_KEY => ToServer::Key(r.key()?),
        TO_SERVER_RESIZE => ToServer::Resize(r.size()?),
        TO_SERVER_READ => ToServer::Read(r.read()?),
        TO_SERVER_TOCK => ToServer::Tock(r.tock()?),
        TO_SERVER_LIST => ToServer::List,
        TO_SERVER_KILL => ToServer::Kill {
            session: r.string()?,
        },
        TO_SERVER_COMMAND => {
            let named = r.bool()?;
            let session = r.string()?;
            let name = r.string()?;
            let mut args = Vec::new();
            for _ in 0..r.len()? {
                args.push(r.string()?);
            }
            ToServer::Command {
                session: named.then_some(session),
                name,
                args,
            }
        }
        _ => return Err(WireError::Malformed("client message discriminant")),
    };
    r.finish()?;
    Ok(msg)
}

pub fn encode_to_client(msg: &ToClient<'_>) -> Vec<u8> {
    let mut w = Writer::new();
    match msg {
        ToClient::Tick(tick) => {
            w.u8(TO_CLIENT_TICK);
            w.tick(tick);
        }
        ToClient::Done(done) => {
            w.u8(TO_CLIENT_DONE);
            w.done(done);
        }
        ToClient::Reading(reading) => {
            w.u8(TO_CLIENT_READING);
            w.reading(reading);
        }
        ToClient::Refused(why) => {
            w.u8(TO_CLIENT_REFUSED);
            w.str(why);
        }
        ToClient::Sessions(names) => {
            w.u8(TO_CLIENT_SESSIONS);
            w.len(names.len());
            for name in *names {
                w.str(name);
            }
        }
        ToClient::Killed(found) => {
            w.u8(TO_CLIENT_KILLED);
            w.bool(*found);
        }
        ToClient::Clip(text) => {
            w.u8(TO_CLIENT_CLIP);
            w.str(text);
        }
        ToClient::Bye(lines) => {
            w.u8(TO_CLIENT_BYE);
            w.len(lines.len());
            for line in *lines {
                w.str(line);
            }
        }
        ToClient::Ran { ok, lines } => {
            w.u8(TO_CLIENT_RAN);
            w.bool(*ok);
            w.len(lines.len());
            for line in *lines {
                w.str(line);
            }
        }
    }
    w.finish()
}

pub fn decode_to_client(bytes: &[u8]) -> Result<OwnedToClient, WireError> {
    let mut r = Reader::new(bytes);
    let msg = match r.u8()? {
        TO_CLIENT_TICK => OwnedToClient::Tick(r.tick()?),
        TO_CLIENT_DONE => OwnedToClient::Done(r.done()?),
        TO_CLIENT_READING => OwnedToClient::Reading(r.reading()?),
        TO_CLIENT_REFUSED => OwnedToClient::Refused(r.string()?),
        TO_CLIENT_SESSIONS => {
            let n = r.len()?;
            let mut names = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                names.push(r.string()?);
            }
            OwnedToClient::Sessions(names)
        }
        TO_CLIENT_KILLED => OwnedToClient::Killed(r.bool()?),
        TO_CLIENT_CLIP => OwnedToClient::Clip(r.string()?),
        TO_CLIENT_BYE => {
            let n = r.len()?;
            let mut lines = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                lines.push(r.string()?);
            }
            OwnedToClient::Bye(lines)
        }
        TO_CLIENT_RAN => {
            let ok = r.bool()?;
            let mut lines = Vec::new();
            for _ in 0..r.len()? {
                lines.push(r.string()?);
            }
            OwnedToClient::Ran { ok, lines }
        }
        _ => return Err(WireError::Malformed("server message discriminant")),
    };
    r.finish()?;
    Ok(msg)
}

impl Writer {
    fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn opt_u64(&mut self, v: Option<u64>) {
        match v {
            None => self.bool(false),
            Some(n) => {
                self.bool(true);
                self.u64(n);
            }
        }
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

    fn tick(&mut self, v: &Tick) {
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

    fn tock(&mut self, v: &Tock) {
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

    fn read(&mut self, v: Read) {
        match v {
            Read::Revision { pane } => {
                self.u8(READ_REVISION);
                self.u32(pane.get());
            }
            Read::Content { pane, last, want } => {
                self.u8(READ_CONTENT);
                self.u32(pane.get());
                self.opt_u64(last);
                self.bool(matches!(want.trailing, Trailing::Pad));
                self.bool(want.wrapped);
            }
            Read::HistoryExtent { pane } => {
                self.u8(READ_HISTORY_EXTENT);
                self.u32(pane.get());
            }
            Read::History {
                pane,
                from,
                count,
                want,
            } => {
                self.u8(READ_HISTORY);
                self.u32(pane.get());
                self.u64(from);
                self.u64(count);
                self.bool(matches!(want.trailing, Trailing::Pad));
                self.bool(want.wrapped);
            }
            Read::Freeze { pane } => {
                self.u8(READ_FREEZE);
                self.u32(pane.get());
            }
            Read::Unfreeze { pane } => {
                self.u8(READ_UNFREEZE);
                self.u32(pane.get());
            }
            Read::IsFrozen { pane } => {
                self.u8(READ_IS_FROZEN);
                self.u32(pane.get());
            }
        }
    }

    fn lines(&mut self, v: &Lines) {
        self.len(v.lines.len());
        for line in &v.lines {
            self.str(line);
        }
        match &v.wrapped {
            None => self.bool(false),
            Some(wrapped) => {
                self.bool(true);
                self.len(wrapped.len());
                for flag in wrapped {
                    self.bool(*flag);
                }
            }
        }
        self.u64(v.revision);
    }

    fn reading(&mut self, v: &Reading) {
        match v {
            Reading::Revision(revision) => {
                self.u8(READING_REVISION);
                self.opt_u64(*revision);
            }
            Reading::Text(text) => {
                self.u8(READING_TEXT);
                match text {
                    None => self.bool(false),
                    Some(text) => {
                        self.bool(true);
                        self.lines(text);
                    }
                }
            }
            Reading::Extent(extent) => {
                self.u8(READING_EXTENT);
                match extent {
                    None => self.bool(false),
                    Some(extent) => {
                        self.bool(true);
                        self.u64(extent.len);
                        self.u64(extent.dropped);
                    }
                }
            }
            Reading::Did(did) => {
                self.u8(READING_DID);
                self.bool(*did);
            }
            Reading::Frozen(frozen) => {
                self.u8(READING_FROZEN);
                match frozen {
                    None => self.bool(false),
                    Some(frozen) => {
                        self.bool(true);
                        self.bool(*frozen);
                    }
                }
            }
        }
    }

    fn done(&mut self, v: &crate::msg::Done<'_>) {
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

    fn frame(&mut self, v: &Frame<'_>) {
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

impl Reader<'_> {
    fn u64(&mut self) -> Result<u64, WireError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn opt_u64(&mut self) -> Result<Option<u64>, WireError> {
        if self.bool()? {
            Ok(Some(self.u64()?))
        } else {
            Ok(None)
        }
    }

    fn opt_string(&mut self) -> Result<Option<String>, WireError> {
        if self.bool()? {
            Ok(Some(self.string()?))
        } else {
            Ok(None)
        }
    }

    fn window(&mut self) -> Result<WindowId, WireError> {
        Ok(WindowId::from(self.u32()?))
    }

    fn opt_window(&mut self) -> Result<Option<WindowId>, WireError> {
        if self.bool()? {
            Ok(Some(self.window()?))
        } else {
            Ok(None)
        }
    }

    fn reserved(&mut self) -> Result<Reserved, WireError> {
        let top = self.u16()?;
        let bottom = self.u16()?;
        let mut reserved = Reserved::horizontal(top, bottom);
        reserved.add(Edge::Left, self.u16()?);
        reserved.add(Edge::Right, self.u16()?);
        Ok(reserved)
    }

    fn split_dir(&mut self) -> Result<SplitDir, WireError> {
        match self.u8()? {
            SPLIT_HORIZONTAL => Ok(SplitDir::Horizontal),
            SPLIT_VERTICAL => Ok(SplitDir::Vertical),
            _ => Err(WireError::Malformed("split direction")),
        }
    }

    fn dir(&mut self) -> Result<Dir, WireError> {
        match self.u8()? {
            DIR_LEFT => Ok(Dir::Left),
            DIR_RIGHT => Ok(Dir::Right),
            DIR_UP => Ok(Dir::Up),
            DIR_DOWN => Ok(Dir::Down),
            _ => Err(WireError::Malformed("direction")),
        }
    }

    fn epoch(&mut self) -> Result<Epoch, WireError> {
        let size = self.size()?;
        let mut panes = Vec::new();
        for _ in 0..self.len()? {
            panes.push(self.pane()?);
        }
        let focus = self.opt_pane()?;
        Ok(Epoch::new(size, panes, focus))
    }

    fn geometry(&mut self) -> Result<Geometry, WireError> {
        let area = self.rect()?;
        let mut panes = Vec::new();
        for _ in 0..self.len()? {
            panes.push((self.pane()?, self.rect()?));
        }
        let focus = self.opt_pane()?;
        let mut windows = Vec::new();
        for _ in 0..self.len()? {
            let id = self.window()?;
            let mut held = Vec::new();
            for _ in 0..self.len()? {
                held.push(self.pane()?);
            }
            let name = self.opt_string()?;
            windows.push(Window {
                id,
                panes: held,
                name,
            });
        }
        Ok(Geometry {
            area,
            panes,
            focus,
            windows,
            window: self.opt_window()?,
        })
    }

    fn pane_info(&mut self) -> Result<BTreeMap<PaneId, PaneInfo>, WireError> {
        let mut info = BTreeMap::new();
        for _ in 0..self.len()? {
            let pane = self.pane()?;
            info.insert(
                pane,
                PaneInfo {
                    command: self.opt_string()?,
                    cwd: self.opt_string()?,
                },
            );
        }
        Ok(info)
    }

    fn event(&mut self) -> Result<Event, WireError> {
        Ok(match self.u8()? {
            EVENT_PANE_OUTPUT => {
                let pane = self.pane()?;
                let n = self.len()?;
                Event::PaneOutput {
                    pane,
                    bytes: self.take(n)?.to_vec(),
                }
            }
            EVENT_RESIZED => Event::Resized { size: self.size()? },
            EVENT_PANE_OPENED => Event::PaneOpened { pane: self.pane()? },
            EVENT_PANE_CLOSED => Event::PaneClosed { pane: self.pane()? },
            EVENT_FOCUS_CHANGED => Event::FocusChanged { pane: self.pane()? },
            EVENT_WINDOW_OPENED => Event::WindowOpened {
                window: self.window()?,
            },
            EVENT_WINDOW_CLOSED => Event::WindowClosed {
                window: self.window()?,
            },
            EVENT_WINDOW_CHANGED => Event::WindowChanged {
                window: self.window()?,
            },
            _ => return Err(WireError::Malformed("event discriminant")),
        })
    }

    fn step(&mut self) -> Result<Step, WireError> {
        Ok(match self.u8()? {
            STEP_GEOMETRY => Step::Geometry(self.geometry()?),
            STEP_BANDS => Step::Bands {
                surface: self.size()?,
                applied: self.reserved()?,
            },
            STEP_EVENT => Step::Event(self.event()?),
            _ => return Err(WireError::Malformed("step discriminant")),
        })
    }

    fn tick(&mut self) -> Result<Tick, WireError> {
        let mut steps = Vec::new();
        for _ in 0..self.len()? {
            steps.push(self.step()?);
        }
        let epoch = self.epoch()?;
        let geometry = self.geometry()?;
        let pane_info = if self.bool()? {
            Some(self.pane_info()?)
        } else {
            None
        };
        let mut revisions = BTreeMap::new();
        for _ in 0..self.len()? {
            revisions.insert(self.pane()?, self.u64()?);
        }
        Ok(Tick {
            steps,
            epoch,
            geometry,
            pane_info,
            revisions,
        })
    }

    fn op(&mut self) -> Result<SessionOp, WireError> {
        Ok(match self.u8()? {
            OP_QUIT => SessionOp::Quit,
            OP_DETACH => SessionOp::Detach,
            OP_PANE_SPLIT => SessionOp::PaneSplit {
                dir: self.split_dir()?,
            },
            OP_PANE_FOCUS => SessionOp::PaneFocus { pane: self.pane()? },
            OP_PANE_RESIZE => SessionOp::PaneResize {
                dir: self.dir()?,
                cells: self.u16()?,
            },
            OP_WINDOW_CREATE => SessionOp::WindowCreate {
                window: self.window()?,
            },
            OP_WINDOW_FOCUS => SessionOp::WindowFocus {
                window: self.window()?,
            },
            OP_WINDOW_RENAME => SessionOp::WindowRename {
                window: self.window()?,
                name: self.opt_string()?,
            },
            _ => return Err(WireError::Malformed("op discriminant")),
        })
    }

    fn tock(&mut self) -> Result<Tock, WireError> {
        let mut ops = Vec::new();
        for _ in 0..self.len()? {
            ops.push((self.string()?, self.op()?));
        }
        let mut overlay = Vec::new();
        for _ in 0..self.len()? {
            overlay.push(self.draw_cmd()?);
        }
        Ok(Tock {
            ops,
            overlay,
            reserved: self.reserved()?,
            gutter: self.u16()?,
            wants_pane_output: self.bool()?,
        })
    }

    fn read(&mut self) -> Result<Read, WireError> {
        Ok(match self.u8()? {
            READ_REVISION => Read::Revision { pane: self.pane()? },
            READ_CONTENT => Read::Content {
                pane: self.pane()?,
                last: self.opt_u64()?,
                want: Want {
                    trailing: if self.bool()? {
                        Trailing::Pad
                    } else {
                        Trailing::Trim
                    },
                    wrapped: self.bool()?,
                },
            },
            READ_HISTORY_EXTENT => Read::HistoryExtent { pane: self.pane()? },
            READ_HISTORY => Read::History {
                pane: self.pane()?,
                from: self.u64()?,
                count: self.u64()?,
                want: Want {
                    trailing: if self.bool()? {
                        Trailing::Pad
                    } else {
                        Trailing::Trim
                    },
                    wrapped: self.bool()?,
                },
            },
            READ_FREEZE => Read::Freeze { pane: self.pane()? },
            READ_UNFREEZE => Read::Unfreeze { pane: self.pane()? },
            READ_IS_FROZEN => Read::IsFrozen { pane: self.pane()? },
            _ => return Err(WireError::Malformed("read discriminant")),
        })
    }

    fn lines(&mut self) -> Result<Lines, WireError> {
        let mut lines = Vec::new();
        for _ in 0..self.len()? {
            lines.push(self.string()?);
        }
        let wrapped = if self.bool()? {
            let mut flags = Vec::new();
            for _ in 0..self.len()? {
                flags.push(self.bool()?);
            }
            Some(flags)
        } else {
            None
        };
        Ok(Lines {
            lines,
            wrapped,
            revision: self.u64()?,
        })
    }

    fn reading(&mut self) -> Result<Reading, WireError> {
        Ok(match self.u8()? {
            READING_REVISION => Reading::Revision(self.opt_u64()?),
            READING_TEXT => Reading::Text(if self.bool()? {
                Some(self.lines()?)
            } else {
                None
            }),
            READING_EXTENT => Reading::Extent(if self.bool()? {
                Some(Extent {
                    len: self.u64()?,
                    dropped: self.u64()?,
                })
            } else {
                None
            }),
            READING_DID => Reading::Did(self.bool()?),
            READING_FROZEN => Reading::Frozen(if self.bool()? {
                Some(self.bool()?)
            } else {
                None
            }),
            _ => return Err(WireError::Malformed("reading discriminant")),
        })
    }

    fn done(&mut self) -> Result<crate::msg::OwnedDone, WireError> {
        let mut steps = Vec::new();
        for _ in 0..self.len()? {
            steps.push(self.step()?);
        }
        let frame = if self.bool()? {
            Some(self.frame()?)
        } else {
            None
        };
        let end = if self.bool()? {
            Some(match self.u8()? {
                END_QUIT => End::Quit,
                END_EOF => End::Eof,
                END_DETACH => End::Detach,
                _ => return Err(WireError::Malformed("end discriminant")),
            })
        } else {
            None
        };
        Ok(crate::msg::OwnedDone { steps, frame, end })
    }

    fn frame(&mut self) -> Result<OwnedFrame, WireError> {
        let size = self.size()?;
        let mut panes = Vec::new();
        for _ in 0..self.len()? {
            panes.push(self.pane_frame()?);
        }
        let rects = self.pane_rects()?;
        let cursor = self.opt_pane()?;
        let mut overlay = Vec::new();
        for _ in 0..self.len()? {
            overlay.push(self.draw_cmd()?);
        }
        Ok(OwnedFrame {
            size,
            panes,
            rects,
            cursor,
            overlay,
        })
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
        // The one command, which is the last thing in a frame: tag, pane, rect, fg, bg.
        let tag = bytes.len() - 12;
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
            OwnedFrame {
                size: Size {
                    rows: self.next() as u16,
                    cols: self.next() as u16,
                },
                panes,
                rects,
                cursor: self.opt_pane(),
                overlay,
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

    fn geometry_of() -> Geometry {
        Geometry {
            area: Rect::new(0, 0, 80, 24),
            panes: vec![
                (PaneId::from(1), Rect::new(0, 0, 40, 24)),
                (PaneId::from(2), Rect::new(40, 0, 40, 24)),
            ],
            focus: Some(PaneId::from(2)),
            windows: vec![
                Window {
                    id: WindowId::from(1),
                    panes: vec![PaneId::from(1), PaneId::from(2)],
                    name: Some("編集".to_string()),
                },
                Window {
                    id: WindowId::from(2),
                    panes: vec![PaneId::from(3)],
                    name: None,
                },
            ],
            window: Some(WindowId::from(1)),
        }
    }

    fn round_trip_to_server(msg: ToServer) {
        let bytes = encode_to_server(&msg);
        assert_eq!(decode_to_server(&bytes), Ok(msg));
    }

    #[test]
    fn an_attach_round_trips() {
        round_trip_to_server(ToServer::Attach(Attach {
            size: Size { rows: 24, cols: 80 },
            session: "work".to_string(),
        }));
    }

    #[test]
    fn a_key_a_resize_and_every_read_round_trip() {
        round_trip_to_server(ToServer::Key(Key::plain(KeyCode::Char('q'))));
        round_trip_to_server(ToServer::Resize(Size { rows: 3, cols: 9 }));
        for read in [
            Read::Revision {
                pane: PaneId::from(1),
            },
            Read::Content {
                pane: PaneId::from(1),
                last: Some(4),
                want: Want {
                    trailing: Trailing::Pad,
                    wrapped: false,
                },
            },
            Read::Content {
                pane: PaneId::from(1),
                last: None,
                want: Want {
                    trailing: Trailing::Trim,
                    wrapped: true,
                },
            },
            Read::HistoryExtent {
                pane: PaneId::from(2),
            },
            Read::History {
                pane: PaneId::from(2),
                from: 3,
                count: u64::MAX,
                want: Want {
                    trailing: Trailing::Trim,
                    wrapped: true,
                },
            },
            Read::Freeze {
                pane: PaneId::from(3),
            },
            Read::Unfreeze {
                pane: PaneId::from(3),
            },
            Read::IsFrozen {
                pane: PaneId::from(3),
            },
        ] {
            round_trip_to_server(ToServer::Read(read));
        }
    }

    /// The bytes did not move when `pad: bool, wrapped: bool` became `want: Want`.
    ///
    /// A `Trailing` has two variants, so it encodes as the one byte the `pad` bool encoded as,
    /// and `Want` is that byte followed by the `wrapped` byte — the order the two were already
    /// written in. The fixture is the encoding as it was before the change, recorded by hand, so
    /// that this is a claim about bytes and not about the code that produces them: if it fails,
    /// `PROTOCOL_VERSION` moves and the design's §4 is wrong.
    #[test]
    fn a_content_read_encodes_the_bytes_it_encoded_before_want() {
        let read = Read::Content {
            pane: PaneId::from(7),
            last: Some(3),
            want: Want {
                trailing: Trailing::Pad,
                wrapped: false,
            },
        };
        let encoded = encode_to_server(&ToServer::Read(read));

        // pad = true, wrapped = false — the last two bytes of the message, unchanged.
        assert_eq!(
            &encoded[encoded.len() - 2..],
            &[1, 0],
            "the two option bytes moved: {encoded:?}"
        );
        assert_eq!(
            decode_to_server(&encoded).expect("decodes"),
            ToServer::Read(read)
        );
    }

    /// `Read::History`'s encode arm changed identically to `Read::Content`'s; `Trailing::Trim`
    /// and `wrapped: true` make its fixture `[0, 1]`, the opposite of `Content`'s `[1, 0]`
    /// above, so a copy-paste error between the two arms fails this rather than passing it.
    #[test]
    fn a_history_read_encodes_the_bytes_it_encoded_before_want() {
        let read = Read::History {
            pane: PaneId::from(7),
            from: 3,
            count: 5,
            want: Want {
                trailing: Trailing::Trim,
                wrapped: true,
            },
        };
        let encoded = encode_to_server(&ToServer::Read(read));

        // pad = false, wrapped = true — the last two bytes of the message, unchanged.
        assert_eq!(
            &encoded[encoded.len() - 2..],
            &[0, 1],
            "the two option bytes moved: {encoded:?}"
        );
        assert_eq!(
            decode_to_server(&encoded).expect("decodes"),
            ToServer::Read(read)
        );
    }

    #[test]
    fn a_tick_carries_its_steps_in_order() {
        let tick = Tick {
            steps: vec![
                Step::Event(Event::PaneOutput {
                    pane: PaneId::from(1),
                    bytes: b"hello\x1b[0m".to_vec(),
                }),
                Step::Geometry(geometry_of()),
                Step::Bands {
                    surface: Size { rows: 24, cols: 80 },
                    applied: Reserved::vertical(3, 4),
                },
                Step::Event(Event::WindowChanged {
                    window: WindowId::from(2),
                }),
                Step::Event(Event::Resized {
                    size: Size { rows: 1, cols: 1 },
                }),
            ],
            epoch: Epoch::new(
                Size { rows: 24, cols: 80 },
                vec![PaneId::from(2), PaneId::from(1)],
                Some(PaneId::from(1)),
            ),
            geometry: geometry_of(),
            revisions: BTreeMap::from([(PaneId::from(1), 7), (PaneId::from(2), 0)]),
            pane_info: Some(BTreeMap::from([
                (
                    PaneId::from(1),
                    PaneInfo {
                        command: Some("vim".to_string()),
                        cwd: None,
                    },
                ),
                (PaneId::from(2), PaneInfo::default()),
            ])),
        };
        let bytes = encode_to_client(&ToClient::Tick(&tick));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Tick(tick)));
    }

    #[test]
    fn a_tick_with_nothing_resampled_says_so() {
        let tick = Tick {
            steps: Vec::new(),
            epoch: Epoch::default(),
            geometry: Geometry::default(),
            pane_info: None,
            revisions: BTreeMap::new(),
        };
        let bytes = encode_to_client(&ToClient::Tick(&tick));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Tick(tick)));
    }

    #[test]
    fn a_tock_round_trips_with_every_op() {
        let tock = Tock {
            ops: vec![
                ("plugin/prefix".to_string(), SessionOp::Quit),
                ("plugin/prefix".to_string(), SessionOp::Detach),
                (
                    "plugin/prefix".to_string(),
                    SessionOp::PaneSplit {
                        dir: SplitDir::Vertical,
                    },
                ),
                (
                    "init".to_string(),
                    SessionOp::PaneFocus {
                        pane: PaneId::from(9),
                    },
                ),
                (
                    "init".to_string(),
                    SessionOp::PaneResize {
                        dir: Dir::Down,
                        cells: 5,
                    },
                ),
                (
                    "init".to_string(),
                    SessionOp::WindowCreate {
                        window: WindowId::from(4),
                    },
                ),
                (
                    "init".to_string(),
                    SessionOp::WindowFocus {
                        window: WindowId::from(4),
                    },
                ),
                (
                    "init".to_string(),
                    SessionOp::WindowRename {
                        window: WindowId::from(4),
                        name: Some("編集".to_string()),
                    },
                ),
                (
                    "init".to_string(),
                    SessionOp::WindowRename {
                        window: WindowId::from(4),
                        name: None,
                    },
                ),
                ("init".to_string(), SessionOp::Detach),
            ],
            overlay: vec![DrawCmd::Text {
                pane: None,
                x: 1,
                y: 2,
                text: "painted".to_string(),
                fg: Color::Idx(3),
                bg: Color::Default,
            }],
            reserved: Reserved::horizontal(1, 0),
            gutter: 2,
            wants_pane_output: true,
        };
        round_trip_to_server(ToServer::Tock(tock));
    }

    #[test]
    fn a_finished_frame_a_reading_and_a_refusal_round_trip() {
        let frame = frame_of(vec![OwnedPaneFrame {
            pane: PaneId::from(1),
            rect: Rect::new(0, 0, 2, 2),
            grid: grid_of(2, 2, &["a", "b", "c", "d"]),
        }]);
        let steps = vec![
            Step::Geometry(geometry_of()),
            Step::Event(Event::PaneOpened {
                pane: PaneId::from(2),
            }),
        ];
        let borrowed = frame.frame();
        let done = crate::msg::Done {
            steps: steps.clone(),
            frame: Some(&borrowed),
            end: Some(End::Quit),
        };
        let bytes = encode_to_client(&ToClient::Done(&done));
        assert_eq!(
            decode_to_client(&bytes),
            Ok(OwnedToClient::Done(crate::msg::OwnedDone {
                steps,
                frame: Some(frame),
                end: Some(End::Quit),
            }))
        );

        for end in [None, Some(End::Quit), Some(End::Eof), Some(End::Detach)] {
            let done = crate::msg::Done {
                steps: Vec::new(),
                frame: None,
                end,
            };
            let bytes = encode_to_client(&ToClient::Done(&done));
            assert_eq!(
                decode_to_client(&bytes),
                Ok(OwnedToClient::Done(crate::msg::OwnedDone {
                    steps: Vec::new(),
                    frame: None,
                    end,
                }))
            );
        }

        for reading in [
            Reading::Revision(Some(12)),
            Reading::Revision(None),
            Reading::Text(Some(Lines {
                lines: vec!["one".to_string(), "two".to_string()],
                wrapped: Some(vec![true, false]),
                revision: 3,
            })),
            Reading::Text(None),
            Reading::Extent(Some(Extent { len: 4, dropped: 1 })),
            Reading::Extent(None),
            Reading::Did(true),
            Reading::Frozen(Some(false)),
            Reading::Frozen(None),
        ] {
            let bytes = encode_to_client(&ToClient::Reading(&reading));
            assert_eq!(
                decode_to_client(&bytes),
                Ok(OwnedToClient::Reading(reading))
            );
        }

        let bytes = encode_to_client(&ToClient::Refused("already attached"));
        assert_eq!(
            decode_to_client(&bytes),
            Ok(OwnedToClient::Refused("already attached".to_string()))
        );
    }

    /// Encoded and decoded inline, the way the `Refused` case beside it is: `ToClient`
    /// borrows and `OwnedToClient` does not, so the two sides of a round trip are not the
    /// same type and there is no helper to hand them both to.
    #[test]
    fn the_two_questions_that_are_not_an_attach_round_trip() {
        round_trip_to_server(ToServer::List);
        round_trip_to_server(ToServer::Kill {
            session: "work".to_string(),
        });

        let names = vec!["default".to_string(), "work".to_string()];
        let bytes = encode_to_client(&ToClient::Sessions(&names));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Sessions(names)));

        for found in [true, false] {
            let bytes = encode_to_client(&ToClient::Killed(found));
            assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Killed(found)));
        }
    }

    #[test]
    fn a_command_and_its_answer_round_trip() {
        let asked = ToServer::Command {
            session: Some("work".to_string()),
            name: "send".to_string(),
            args: vec!["%4".to_string(), "cargo test".to_string()],
        };
        assert_eq!(decode_to_server(&encode_to_server(&asked)), Ok(asked));

        let lines = vec!["%1 @1 80x24 bash /home/kohei".to_string()];
        let answered = encode_to_client(&ToClient::Ran {
            ok: true,
            lines: &lines,
        });
        assert_eq!(
            decode_to_client(&answered),
            Ok(OwnedToClient::Ran { ok: true, lines })
        );
    }

    /// A command with no session named is the default session's, and `None` has to survive the
    /// wire as `None` rather than as an empty name: `""` is a name the door would refuse.
    #[test]
    fn a_command_with_no_session_named_keeps_its_none() {
        let asked = ToServer::Command {
            session: None,
            name: "ls".to_string(),
            args: Vec::new(),
        };
        assert_eq!(decode_to_server(&encode_to_server(&asked)), Ok(asked));
    }

    #[test]
    fn the_clipboard_and_the_goodbye_round_trip() {
        let bytes = encode_to_client(&ToClient::Clip("yanked 日本語"));
        assert_eq!(
            decode_to_client(&bytes),
            Ok(OwnedToClient::Clip("yanked 日本語".to_string()))
        );

        let lines = vec!["a: one".to_string(), String::new()];
        let bytes = encode_to_client(&ToClient::Bye(&lines));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Bye(lines)));

        let bytes = encode_to_client(&ToClient::Bye(&[]));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Bye(Vec::new())));
    }

    #[test]
    fn a_message_nobody_sent_is_malformed_rather_than_a_panic() {
        assert_eq!(
            decode_to_server(&[250]),
            Err(WireError::Malformed("client message discriminant"))
        );
        assert_eq!(
            decode_to_client(&[250]),
            Err(WireError::Malformed("server message discriminant"))
        );
        assert_eq!(decode_to_server(&[]), Err(WireError::Truncated));
        for tag in 0u8..5 {
            for len in 1..12 {
                let bytes: Vec<u8> = std::iter::once(tag)
                    .chain(std::iter::repeat_n(0, len))
                    .collect();
                let _ = decode_to_server(&bytes);
                let _ = decode_to_client(&bytes);
            }
        }
    }
}

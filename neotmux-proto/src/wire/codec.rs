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

use super::tags::*;
use crate::Key;
use crate::PROTOCOL_VERSION;
use crate::screen::{Frame, OwnedFrame};
use crate::wire::Reader;
use crate::wire::WireError;
use crate::wire::Writer;
use crate::wire::{Attach, OwnedToClient, ToClient, ToServer};

/// The length of the handshake, which is the one message with a fixed size.
pub const HELLO_LEN: usize = 4;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Event;
    use crate::KeyCode;
    use crate::LineMode;
    use crate::Modifiers;
    use crate::PaneId;
    use crate::PaneInfo;
    use crate::PaneTty;
    use crate::WindowId;
    use crate::geom::Color;
    use crate::geom::Dir;
    use crate::geom::Geometry;
    use crate::geom::GridSnapshot;
    use crate::geom::PaneRects;
    use crate::geom::Reserved;
    use crate::geom::Size;
    use crate::geom::SplitDir;
    use crate::geom::Window;
    use crate::screen::DrawCmd;
    use crate::screen::Epoch;
    use crate::screen::OwnedPaneFrame;
    use crate::screen::Rect;
    use crate::wire::End;
    use crate::wire::SessionOp;
    use crate::wire::Step;
    use crate::wire::Tick;
    use crate::wire::Tock;
    use std::collections::BTreeMap;

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
    fn a_key_and_a_resize_round_trip() {
        round_trip_to_server(ToServer::Key(Key::plain(KeyCode::Char('q'))));
        round_trip_to_server(ToServer::Resize(Size { rows: 3, cols: 9 }));
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
            pane_tty: BTreeMap::from([
                (
                    PaneId::from(1),
                    PaneTty {
                        cursor: (3, 7),
                        cursor_visible: true,
                        alternate: false,
                        application_cursor: true,
                        line: Some(LineMode {
                            canonical: true,
                            echo: false,
                        }),
                    },
                ),
                (PaneId::from(2), PaneTty::default()),
            ]),
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
            pane_tty: BTreeMap::new(),
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
                    "plugin/slime".to_string(),
                    SessionOp::PaneSend {
                        pane: PaneId::from(3),
                        text: "print('日本語も')\n".to_string(),
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
        let done = crate::wire::Done {
            steps: steps.clone(),
            frame: Some(&borrowed),
            end: Some(End::Quit),
        };
        let bytes = encode_to_client(&ToClient::Done(&done));
        assert_eq!(
            decode_to_client(&bytes),
            Ok(OwnedToClient::Done(crate::wire::OwnedDone {
                steps,
                frame: Some(frame),
                end: Some(End::Quit),
            }))
        );

        for end in [None, Some(End::Quit), Some(End::Eof), Some(End::Detach)] {
            let done = crate::wire::Done {
                steps: Vec::new(),
                frame: None,
                end,
            };
            let bytes = encode_to_client(&ToClient::Done(&done));
            assert_eq!(
                decode_to_client(&bytes),
                Ok(OwnedToClient::Done(crate::wire::OwnedDone {
                    steps: Vec::new(),
                    frame: None,
                    end,
                }))
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

use super::tags::*;
use crate::screen::{OwnedFrame, OwnedPaneFrame};
use crate::wire::WireError;
use crate::wire::{End, SessionOp, Step, Tick, Tock};
use crate::{
    Color, Dir, DrawCmd, Edge, Epoch, Event, Geometry, GridSnapshot, LineMode, PaneId, PaneInfo,
    PaneRects, PaneTty, Rect, Reserved, Size, SnapCell, SplitDir, Window, WindowId,
};
use crate::{Key, KeyCode, Modifiers};
use std::collections::BTreeMap;

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn finish(self) -> Result<(), WireError> {
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

    pub(crate) fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn bool(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::Malformed("bool")),
        }
    }

    /// A count is never used to reserve: every loop over one reads its items, and a count
    /// larger than the bytes left runs out inside the loop instead of allocating for a
    /// length nothing could have sent.
    pub(crate) fn len(&mut self) -> Result<usize, WireError> {
        Ok(self.u32()? as usize)
    }

    fn char(&mut self) -> Result<char, WireError> {
        char::from_u32(self.u32()?).ok_or(WireError::Malformed("char"))
    }

    pub(crate) fn string(&mut self) -> Result<String, WireError> {
        let n = self.len()?;
        let bytes = self.take(n)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| WireError::Malformed("utf-8"))
    }

    pub(crate) fn size(&mut self) -> Result<Size, WireError> {
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
                bold: self.u8()? != 0,
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

    pub(crate) fn key(&mut self) -> Result<Key, WireError> {
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

impl Reader<'_> {
    fn u64(&mut self) -> Result<u64, WireError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
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

    pub(crate) fn tick(&mut self) -> Result<Tick, WireError> {
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
        let mut pane_tty = BTreeMap::new();
        for _ in 0..self.len()? {
            let pane = self.pane()?;
            let cursor = (self.u16()?, self.u16()?);
            let cursor_visible = self.bool()?;
            let alternate = self.bool()?;
            let application_cursor = self.bool()?;
            let line = if self.bool()? {
                Some(LineMode {
                    canonical: self.bool()?,
                    echo: self.bool()?,
                })
            } else {
                None
            };
            pane_tty.insert(
                pane,
                PaneTty {
                    cursor,
                    cursor_visible,
                    alternate,
                    application_cursor,
                    line,
                },
            );
        }
        let mut revisions = BTreeMap::new();
        for _ in 0..self.len()? {
            revisions.insert(self.pane()?, self.u64()?);
        }
        Ok(Tick {
            steps,
            epoch,
            geometry,
            pane_info,
            pane_tty,
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
            OP_PANE_SEND => SessionOp::PaneSend {
                pane: self.pane()?,
                text: self.string()?,
            },
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

    pub(crate) fn tock(&mut self) -> Result<Tock, WireError> {
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

    pub(crate) fn done(&mut self) -> Result<crate::wire::OwnedDone, WireError> {
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
        Ok(crate::wire::OwnedDone { steps, frame, end })
    }

    pub(crate) fn frame(&mut self) -> Result<OwnedFrame, WireError> {
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

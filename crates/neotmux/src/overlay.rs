use std::collections::HashMap;

use neotmux_core::proto::{Color as PColor, DrawCmd, PaneId, Rect as PRect, StatusFragment};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::color;

/// Where each pane currently sits, in surface-local coordinates (the same
/// space `LuaSubsystem::begin_frame` was given).
pub type PaneRects = HashMap<PaneId, PRect>;

#[derive(Clone, Copy)]
struct Colors {
    fg: PColor,
    bg: PColor,
}

/// The local coordinate frame a command's (x, y) is relative to: either a
/// pane's rect, or the whole surface for `pane: None`.
fn bounds(panes: &PaneRects, area: Rect, pane: Option<PaneId>) -> Option<PRect> {
    match pane {
        Some(id) => panes.get(&id).copied(),
        None => Some(PRect::new(0, 0, area.width, area.height)),
    }
}

fn draw_char(
    buf: &mut Buffer,
    area: Rect,
    bound: PRect,
    pos: (u16, u16),
    ch: char,
    colors: Colors,
) {
    let (x, y) = pos;
    if x >= bound.w || y >= bound.h {
        return;
    }
    let ax = area.x + bound.x + x;
    let ay = area.y + bound.y + y;
    if ax >= area.x + area.width || ay >= area.y + area.height {
        return;
    }
    if let Some(cell) = buf.cell_mut((ax, ay)) {
        let mut tmp = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut tmp));
        cell.set_fg(color(colors.fg));
        cell.set_bg(color(colors.bg));
    }
}

fn draw_text(
    buf: &mut Buffer,
    area: Rect,
    bound: PRect,
    pos: (u16, u16),
    text: &str,
    colors: Colors,
) {
    let (x, y) = pos;
    for (cx, ch) in (x..).zip(text.chars()) {
        if cx >= bound.w {
            break;
        }
        draw_char(buf, area, bound, (cx, y), ch, colors);
    }
}

fn draw_rect(buf: &mut Buffer, area: Rect, bound: PRect, rect: PRect, colors: Colors) {
    let x0 = rect.x.min(bound.w);
    let y0 = rect.y.min(bound.h);
    let x1 = rect.x.saturating_add(rect.w).min(bound.w);
    let y1 = rect.y.saturating_add(rect.h).min(bound.h);
    for y in y0..y1 {
        for x in x0..x1 {
            draw_char(buf, area, bound, (x, y), ' ', colors);
        }
    }
}

fn draw_line(
    buf: &mut Buffer,
    area: Rect,
    bound: PRect,
    from: (u16, u16),
    to: (u16, u16),
    fg: PColor,
) {
    let colors = Colors {
        fg,
        bg: PColor::Default,
    };
    let (x0, y0) = (i32::from(from.0), i32::from(from.1));
    let (x1, y1) = (i32::from(to.0), i32::from(to.1));
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        if x >= 0 && y >= 0 {
            draw_char(buf, area, bound, (x as u16, y as u16), '#', colors);
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Draws every plugin's overlay commands for this frame, clipping each one to
/// its pane's rect (or to the surface for `pane: None`), and silently
/// ignoring a command naming a pane that closed during this same frame.
pub fn apply(frame: &mut Frame, area: Rect, cmds: &[DrawCmd], panes: &PaneRects) {
    let buf = frame.buffer_mut();
    for cmd in cmds {
        match cmd {
            DrawCmd::Text {
                pane,
                x,
                y,
                text,
                fg,
                bg,
            } => {
                let Some(bound) = bounds(panes, area, *pane) else {
                    continue;
                };
                draw_text(
                    buf,
                    area,
                    bound,
                    (*x, *y),
                    text,
                    Colors { fg: *fg, bg: *bg },
                );
            }
            DrawCmd::Rect { pane, rect, fg, bg } => {
                let Some(bound) = bounds(panes, area, *pane) else {
                    continue;
                };
                draw_rect(buf, area, bound, *rect, Colors { fg: *fg, bg: *bg });
            }
            DrawCmd::Line { pane, from, to, fg } => {
                let Some(bound) = bounds(panes, area, *pane) else {
                    continue;
                };
                draw_line(buf, area, bound, *from, *to, *fg);
            }
            DrawCmd::Marker {
                pane,
                x,
                y,
                text,
                fg,
            } => {
                let Some(bound) = bounds(panes, area, *pane) else {
                    continue;
                };
                let colors = Colors {
                    fg: *fg,
                    bg: PColor::Default,
                };
                draw_text(buf, area, bound, (*x, *y), text, colors);
            }
            // Non-exhaustive enum: an unknown future variant is silently skipped.
            _ => {}
        }
    }
}

/// Renders the status fragments left to right across the status row.
pub fn draw_status(frame: &mut Frame, area: Rect, fragments: &[StatusFragment]) {
    if area.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let bound = PRect::new(0, 0, area.width, area.height);
    let mut x = 0u16;
    for fragment in fragments {
        if x >= bound.w {
            break;
        }
        let colors = Colors {
            fg: fragment.fg,
            bg: fragment.bg,
        };
        draw_text(buf, area, bound, (x, 0), &fragment.text, colors);
        x += u16::try_from(fragment.text.chars().count()).unwrap_or(bound.w);
    }
}

#[cfg(test)]
mod tests {
    use neotmux_core::proto::Color;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn render(
        area_size: (u16, u16),
        body: impl FnOnce(&mut Frame, Rect),
    ) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(area_size.0, area_size.1)).unwrap();
        terminal.draw(|frame| body(frame, frame.area())).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn a_surface_command_draws_at_the_surface_origin() {
        let cmds = vec![DrawCmd::Text {
            pane: None,
            x: 0,
            y: 0,
            text: "OVERLAY".to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }];
        let panes = PaneRects::new();
        let buf = render((10, 3), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(0, 0)].symbol(), "O");
        assert_eq!(buf[(6, 0)].symbol(), "Y");
    }

    #[test]
    fn a_command_for_a_pane_is_offset_by_that_panes_rect() {
        let pane = PaneId::from(1);
        let cmds = vec![DrawCmd::Text {
            pane: Some(pane),
            x: 1,
            y: 0,
            text: "X".to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }];
        let mut panes = PaneRects::new();
        panes.insert(pane, PRect::new(2, 1, 5, 2));
        let buf = render((10, 4), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(3, 1)].symbol(), "X");
    }

    #[test]
    fn a_command_for_a_closed_pane_is_ignored() {
        let pane = PaneId::from(1);
        let cmds = vec![DrawCmd::Text {
            pane: Some(pane),
            x: 0,
            y: 0,
            text: "X".to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }];
        let panes = PaneRects::new();
        let buf = render((10, 4), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }

    #[test]
    fn text_is_clipped_to_its_bound() {
        let pane = PaneId::from(1);
        let cmds = vec![DrawCmd::Text {
            pane: Some(pane),
            x: 0,
            y: 0,
            text: "HELLO".to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }];
        let mut panes = PaneRects::new();
        panes.insert(pane, PRect::new(0, 0, 3, 1));
        let buf = render((10, 2), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(2, 0)].symbol(), "L");
        assert_eq!(buf[(3, 0)].symbol(), " ");
    }

    #[test]
    fn status_fragments_are_drawn_left_to_right() {
        let fragments = vec![
            StatusFragment::new("AA", Color::Default, Color::Default),
            StatusFragment::new("BB", Color::Default, Color::Default),
        ];
        let buf = render((10, 1), |frame, area| draw_status(frame, area, &fragments));
        assert_eq!(buf[(0, 0)].symbol(), "A");
        assert_eq!(buf[(2, 0)].symbol(), "B");
    }
}

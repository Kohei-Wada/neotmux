use neotmux_proto::{Color as PColor, DrawCmd, PaneId, PaneRects, Rect as PRect};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::draw::color;

#[derive(Clone, Copy)]
struct Colors {
    fg: PColor,
    bg: PColor,
}

/// A tint's colours: a channel left out is a channel the cell keeps.
#[derive(Clone, Copy)]
struct TintColors {
    fg: Option<PColor>,
    bg: Option<PColor>,
}

/// The local coordinate frame a command's (x, y) is relative to: either a
/// pane's rect, or the whole surface for `pane: None`. Pane rects are in surface
/// coordinates too, so both are offsets into the same `area` — which is the whole terminal,
/// reserved bands included, so a `pane: nil` draw can reach one.
fn bounds(panes: &PaneRects, area: Rect, pane: Option<PaneId>) -> Option<PRect> {
    match pane {
        Some(id) => panes.get(&id).copied(),
        None => Some(PRect::new(0, 0, area.width, area.height)),
    }
}

/// The one cell a local (x, y) names, or nothing when it falls outside its bound or off the
/// surface. Every command goes through here, so clipping is decided once.
fn cell_at(
    buf: &mut Buffer,
    area: Rect,
    bound: PRect,
    pos: (u16, u16),
) -> Option<&mut ratatui::buffer::Cell> {
    let (x, y) = pos;
    if x >= bound.w || y >= bound.h {
        return None;
    }
    let ax = area.x + bound.x + x;
    let ay = area.y + bound.y + y;
    if ax >= area.x + area.width || ay >= area.y + area.height {
        return None;
    }
    buf.cell_mut((ax, ay))
}

fn draw_char(
    buf: &mut Buffer,
    area: Rect,
    bound: PRect,
    pos: (u16, u16),
    ch: char,
    colors: Colors,
) {
    if let Some(cell) = cell_at(buf, area, bound, pos) {
        let mut tmp = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut tmp));
        cell.set_fg(color(colors.fg));
        cell.set_bg(color(colors.bg));
    }
}

fn tint_cell(buf: &mut Buffer, area: Rect, bound: PRect, pos: (u16, u16), colors: TintColors) {
    if let Some(cell) = cell_at(buf, area, bound, pos) {
        if let Some(fg) = colors.fg {
            cell.set_fg(color(fg));
        }
        if let Some(bg) = colors.bg {
            cell.set_bg(color(bg));
        }
    }
}

/// A run advances by what each character is worth on a terminal, not by one per `char`: a
/// label with a wide character in it would otherwise put every glyph after it one column
/// early and overrun its bound by the difference.
fn draw_text(
    buf: &mut Buffer,
    area: Rect,
    bound: PRect,
    pos: (u16, u16),
    text: &str,
    colors: Colors,
) {
    let (x, y) = pos;
    let mut cx = x;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        // A combining mark belongs to the cell before it and claims none of its own. Nothing
        // here can attach it, so it is dropped rather than given a cell it would corrupt.
        if width == 0 {
            continue;
        }
        if cx.saturating_add(width) > bound.w {
            break;
        }
        draw_char(buf, area, bound, (cx, y), ch, colors);
        // ratatui reads a wide glyph as owning the cells after it, which have to be empty.
        for tail in 1..width {
            if let Some(cell) = cell_at(buf, area, bound, (cx + tail, y)) {
                cell.set_symbol("");
                cell.set_fg(color(colors.fg));
                cell.set_bg(color(colors.bg));
            }
        }
        cx += width;
    }
}

/// The cells of a rectangle that are inside `bound`, in reading order.
fn cells_of(bound: PRect, rect: PRect) -> impl Iterator<Item = (u16, u16)> {
    let x0 = rect.x.min(bound.w);
    let y0 = rect.y.min(bound.h);
    let x1 = rect.x.saturating_add(rect.w).min(bound.w);
    let y1 = rect.y.saturating_add(rect.h).min(bound.h);
    (y0..y1).flat_map(move |y| (x0..x1).map(move |x| (x, y)))
}

fn draw_rect(buf: &mut Buffer, area: Rect, bound: PRect, rect: PRect, colors: Colors) {
    for pos in cells_of(bound, rect) {
        draw_char(buf, area, bound, pos, ' ', colors);
    }
}

fn draw_tint(buf: &mut Buffer, area: Rect, bound: PRect, rect: PRect, colors: TintColors) {
    for pos in cells_of(bound, rect) {
        tint_cell(buf, area, bound, pos, colors);
    }
}

fn draw_line(
    buf: &mut Buffer,
    area: Rect,
    bound: PRect,
    from: (u16, u16),
    to: (u16, u16),
    glyph: char,
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
            draw_char(buf, area, bound, (x as u16, y as u16), glyph, colors);
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

/// Draws every source's overlay commands for this frame, clipping each one to
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
            DrawCmd::Tint { pane, rect, fg, bg } => {
                let Some(bound) = bounds(panes, area, *pane) else {
                    continue;
                };
                draw_tint(buf, area, bound, *rect, TintColors { fg: *fg, bg: *bg });
            }
            DrawCmd::Line {
                pane,
                from,
                to,
                glyph,
                fg,
            } => {
                let Some(bound) = bounds(panes, area, *pane) else {
                    continue;
                };
                draw_line(buf, area, bound, *from, *to, *glyph, *fg);
            }
            // Non-exhaustive enum: an unknown future variant is silently skipped.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use neotmux_proto::Color;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color as RColor;

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

    /// A wide character is worth two columns, so what follows it lands two columns along.
    /// The cell it covers is marked empty, which is ratatui's signal to write nothing there —
    /// so at the backend that column is simply never written.
    #[test]
    fn a_wide_character_takes_two_columns() {
        let cmds = vec![DrawCmd::Text {
            pane: None,
            x: 0,
            y: 0,
            text: "\u{5e45}X".to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }];
        let panes = PaneRects::new();
        let buf = render((10, 2), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(0, 0)].symbol(), "\u{5e45}");
        assert_eq!(buf[(2, 0)].symbol(), "X");
        assert_ne!(
            buf[(1, 0)].symbol(),
            "X",
            "the run advanced one column per char"
        );
    }

    /// Half a wide character is not a character. One that would straddle the edge of its
    /// bound is dropped rather than drawn into a single cell.
    #[test]
    fn a_wide_character_that_does_not_fit_is_not_drawn() {
        let pane = PaneId::from(1);
        let cmds = vec![DrawCmd::Text {
            pane: Some(pane),
            x: 0,
            y: 0,
            text: "A\u{5e45}".to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }];
        let mut panes = PaneRects::new();
        panes.insert(pane, PRect::new(0, 0, 2, 1));
        let buf = render((10, 2), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(0, 0)].symbol(), "A");
        assert_eq!(buf[(1, 0)].symbol(), " ");
    }

    fn line(from: (u16, u16), to: (u16, u16)) -> Vec<DrawCmd> {
        glyph_line(from, to, '#')
    }

    fn glyph_line(from: (u16, u16), to: (u16, u16), glyph: char) -> Vec<DrawCmd> {
        vec![DrawCmd::Line {
            pane: None,
            from,
            to,
            glyph,
            fg: Color::Default,
        }]
    }

    /// A box-drawing glyph is three bytes and one cell. Nothing on the path may measure it
    /// in bytes: the cell holds the whole character and the next cell is the next step.
    #[test]
    fn a_multi_byte_glyph_occupies_one_cell_per_step() {
        let cmds = glyph_line((1, 0), (3, 0), '\u{2500}');
        let panes = PaneRects::new();
        let buf = render((10, 2), |frame, area| apply(frame, area, &cmds, &panes));
        for x in 1..=3 {
            assert_eq!(buf[(x, 0)].symbol(), "\u{2500}", "x={x}");
        }
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(buf[(4, 0)].symbol(), " ");
    }

    #[test]
    fn a_horizontal_line_is_drawn_between_its_endpoints() {
        let cmds = line((1, 2), (4, 2));
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        for x in 1..=4 {
            assert_eq!(buf[(x, 2)].symbol(), "#", "x={x}");
        }
        assert_eq!(buf[(0, 2)].symbol(), " ");
        assert_eq!(buf[(5, 2)].symbol(), " ");
    }

    #[test]
    fn a_vertical_line_is_drawn_between_its_endpoints() {
        let cmds = line((2, 0), (2, 3));
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        for y in 0..=3 {
            assert_eq!(buf[(2, y)].symbol(), "#", "y={y}");
        }
        assert_eq!(buf[(2, 4)].symbol(), " ");
    }

    #[test]
    fn a_diagonal_line_going_down_right_is_drawn() {
        let cmds = line((0, 0), (3, 3));
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        for i in 0..=3 {
            assert_eq!(buf[(i, i)].symbol(), "#", "i={i}");
        }
    }

    #[test]
    fn a_diagonal_line_going_up_right_is_drawn() {
        let cmds = line((0, 3), (3, 0));
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        for i in 0..=3u16 {
            assert_eq!(buf[(i, 3 - i)].symbol(), "#", "i={i}");
        }
    }

    #[test]
    fn reversed_endpoints_draw_the_same_diagonal() {
        let cmds = line((3, 3), (0, 0));
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        for i in 0..=3 {
            assert_eq!(buf[(i, i)].symbol(), "#", "i={i}");
        }
    }

    #[test]
    fn a_rect_command_fills_its_area() {
        let cmds = vec![DrawCmd::Rect {
            pane: None,
            rect: PRect::new(1, 1, 3, 2),
            fg: Color::Default,
            bg: Color::Idx(2),
        }];
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        for y in 1..3 {
            for x in 1..4 {
                assert_eq!(buf[(x, y)].symbol(), " ", "x={x} y={y}");
                assert_eq!(buf[(x, y)].bg, RColor::Indexed(2), "x={x} y={y}");
            }
        }
        assert_eq!(buf[(0, 1)].bg, RColor::Reset);
        assert_eq!(buf[(4, 1)].bg, RColor::Reset);
    }

    #[test]
    fn a_rect_straddling_its_bounds_edges_is_clipped() {
        let pane = PaneId::from(1);
        let cmds = vec![DrawCmd::Rect {
            pane: Some(pane),
            rect: PRect::new(3, 1, 10, 10),
            fg: Color::Default,
            bg: Color::Idx(4),
        }];
        let mut panes = PaneRects::new();
        panes.insert(pane, PRect::new(0, 0, 5, 3));
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(3, 1)].bg, RColor::Indexed(4));
        assert_eq!(buf[(4, 2)].bg, RColor::Indexed(4));
        assert_eq!(buf[(5, 1)].bg, RColor::Reset, "past the pane's right edge");
        assert_eq!(buf[(3, 3)].bg, RColor::Reset, "past the pane's bottom edge");
    }

    fn tint(rect: PRect, fg: Option<Color>, bg: Option<Color>) -> DrawCmd {
        DrawCmd::Tint {
            pane: None,
            rect,
            fg,
            bg,
        }
    }

    /// The whole point of a tint: the text under it is still there to read.
    #[test]
    fn a_tint_recolours_a_cell_and_leaves_its_glyph_alone() {
        let cmds = vec![
            DrawCmd::Text {
                pane: None,
                x: 0,
                y: 0,
                text: "abc".to_string(),
                fg: Color::Idx(1),
                bg: Color::Idx(2),
            },
            tint(
                PRect::new(0, 0, 3, 1),
                Some(Color::Idx(7)),
                Some(Color::Idx(4)),
            ),
        ];
        let panes = PaneRects::new();
        let buf = render((10, 2), |frame, area| apply(frame, area, &cmds, &panes));
        for (x, ch) in "abc".chars().enumerate() {
            let x = x as u16;
            assert_eq!(buf[(x, 0)].symbol(), ch.to_string(), "x={x}");
            assert_eq!(buf[(x, 0)].fg, RColor::Indexed(7), "x={x}");
            assert_eq!(buf[(x, 0)].bg, RColor::Indexed(4), "x={x}");
        }
    }

    /// A selection changes the background and keeps whatever coloured the text.
    #[test]
    fn a_channel_the_tint_leaves_out_is_a_channel_the_cell_keeps() {
        let cmds = vec![
            DrawCmd::Text {
                pane: None,
                x: 0,
                y: 0,
                text: "x".to_string(),
                fg: Color::Idx(3),
                bg: Color::Idx(5),
            },
            tint(PRect::new(0, 0, 1, 1), None, Some(Color::Idx(6))),
        ];
        let panes = PaneRects::new();
        let buf = render((10, 2), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(0, 0)].symbol(), "x");
        assert_eq!(buf[(0, 0)].fg, RColor::Indexed(3), "the fg was not named");
        assert_eq!(buf[(0, 0)].bg, RColor::Indexed(6));
    }

    /// A cell nothing has written is a blank with the terminal's own colours, and tinting it
    /// recolours that blank: the tint shows, and no glyph appears where there was none.
    #[test]
    fn tinting_a_cell_nothing_has_written_recolours_the_blank() {
        let cmds = vec![tint(PRect::new(2, 1, 1, 1), None, Some(Color::Idx(8)))];
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(2, 1)].symbol(), " ");
        assert_eq!(buf[(2, 1)].bg, RColor::Indexed(8));
    }

    #[test]
    fn a_tint_straddling_its_bounds_edges_is_clipped() {
        let pane = PaneId::from(1);
        let cmds = vec![DrawCmd::Tint {
            pane: Some(pane),
            rect: PRect::new(3, 1, 10, 10),
            fg: None,
            bg: Some(Color::Idx(4)),
        }];
        let mut panes = PaneRects::new();
        panes.insert(pane, PRect::new(0, 0, 5, 3));
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(3, 1)].bg, RColor::Indexed(4));
        assert_eq!(buf[(5, 1)].bg, RColor::Reset, "past the pane's right edge");
        assert_eq!(buf[(3, 3)].bg, RColor::Reset, "past the pane's bottom edge");
    }

    #[test]
    fn a_tint_naming_no_colour_changes_nothing() {
        let cmds = vec![
            DrawCmd::Text {
                pane: None,
                x: 0,
                y: 0,
                text: "q".to_string(),
                fg: Color::Idx(1),
                bg: Color::Idx(2),
            },
            tint(PRect::new(0, 0, 1, 1), None, None),
        ];
        let panes = PaneRects::new();
        let buf = render((10, 2), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(0, 0)].symbol(), "q");
        assert_eq!(buf[(0, 0)].fg, RColor::Indexed(1));
        assert_eq!(buf[(0, 0)].bg, RColor::Indexed(2));
    }

    #[test]
    fn a_zero_sized_tint_changes_nothing() {
        let cmds = vec![tint(PRect::new(2, 2, 0, 0), None, Some(Color::Idx(9)))];
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(2, 2)].bg, RColor::Reset);
    }

    #[test]
    fn a_tint_for_a_closed_pane_is_ignored() {
        let cmds = vec![DrawCmd::Tint {
            pane: Some(PaneId::from(1)),
            rect: PRect::new(0, 0, 2, 2),
            fg: None,
            bg: Some(Color::Idx(9)),
        }];
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(0, 0)].bg, RColor::Reset);
    }

    #[test]
    fn a_zero_sized_rect_draws_nothing() {
        let cmds = vec![DrawCmd::Rect {
            pane: None,
            rect: PRect::new(2, 2, 0, 0),
            fg: Color::Default,
            bg: Color::Idx(9),
        }];
        let panes = PaneRects::new();
        let buf = render((10, 5), |frame, area| apply(frame, area, &cmds, &panes));
        assert_eq!(buf[(2, 2)].bg, RColor::Reset);
    }
}

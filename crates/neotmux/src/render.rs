use neotmux_core::proto::{Color as PColor, GridSnapshot};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};

pub(crate) fn color(c: PColor) -> Color {
    match c {
        PColor::Default => Color::Reset,
        PColor::Idx(i) => Color::Indexed(i),
        PColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        _ => Color::Reset,
    }
}

/// Splits the terminal area into the pane area and the status row. The
/// status row only exists (and only costs a row) when there is a status to
/// show; otherwise the pane gets the whole area, exactly as before.
pub fn layout(area: Rect, has_status: bool) -> (Rect, Rect) {
    if has_status && area.height > 1 {
        let pane = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height - 1,
        };
        let status = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        (pane, status)
    } else {
        (
            area,
            Rect {
                x: area.x,
                y: area.y + area.height,
                width: area.width,
                height: 0,
            },
        )
    }
}

pub fn draw_grid(frame: &mut Frame, area: Rect, grid: &GridSnapshot) {
    let buf = frame.buffer_mut();
    let rows = grid.size.rows.min(area.height);
    let cols = grid.size.cols.min(area.width);

    for row in 0..rows {
        for col in 0..cols {
            let Some(src) = grid.cell(row, col) else {
                continue;
            };
            if src.wide_continuation {
                continue;
            }
            let Some(dst) = buf.cell_mut((area.x + col, area.y + row)) else {
                continue;
            };
            dst.set_symbol(if src.text.is_empty() { " " } else { &src.text });
            dst.set_fg(color(src.fg));
            dst.set_bg(color(src.bg));
            let mut modifier = Modifier::empty();
            modifier.set(Modifier::BOLD, src.bold);
            modifier.set(Modifier::ITALIC, src.italic);
            modifier.set(Modifier::UNDERLINED, src.underline);
            modifier.set(Modifier::REVERSED, src.inverse);
            dst.modifier = modifier;
        }
    }

    if !grid.hide_cursor {
        let (crow, ccol) = grid.cursor;
        if crow < area.height && ccol < area.width {
            frame.set_cursor_position(Position::new(area.x + ccol, area.y + crow));
        }
    }
}

#[cfg(test)]
mod tests {
    use neotmux_core::proto::{Color as PColor, GridSnapshot, Size};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    use super::draw_grid;

    fn render(grid: &GridSnapshot, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| draw_grid(frame, frame.area(), grid))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn writes_characters_at_their_coordinates() {
        let mut g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        g.cell_mut(0, 0).unwrap().text.push('X');
        g.cell_mut(1, 2).unwrap().text.push('Y');
        let buf = render(&g, 3, 2);
        assert_eq!(buf[(0, 0)].symbol(), "X");
        assert_eq!(buf[(2, 1)].symbol(), "Y");
        assert_eq!(buf[(1, 0)].symbol(), " ");
    }

    #[test]
    fn carries_colour_and_bold() {
        let mut g = GridSnapshot::new(Size { rows: 1, cols: 1 });
        let cell = g.cell_mut(0, 0).unwrap();
        cell.text.push('R');
        cell.fg = PColor::Idx(1);
        cell.bg = PColor::Rgb(1, 2, 3);
        cell.bold = true;
        let buf = render(&g, 1, 1);
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(1));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(1, 2, 3));
        assert!(
            buf[(0, 0)]
                .modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }

    #[test]
    fn no_status_gives_the_pane_the_whole_area() {
        let area = Rect::new(0, 0, 80, 24);
        let (pane, status) = super::layout(area, false);
        assert_eq!(pane, area);
        assert_eq!(status.height, 0);
    }

    #[test]
    fn a_status_reserves_the_bottom_row() {
        let area = Rect::new(0, 0, 80, 24);
        let (pane, status) = super::layout(area, true);
        assert_eq!(pane, Rect::new(0, 0, 80, 23));
        assert_eq!(status, Rect::new(0, 23, 80, 1));
    }

    #[test]
    fn a_single_row_area_cannot_spare_a_status_row() {
        let area = Rect::new(0, 0, 80, 1);
        let (pane, status) = super::layout(area, true);
        assert_eq!(pane, area);
        assert_eq!(status.height, 0);
    }

    #[test]
    fn clips_to_the_area() {
        let mut g = GridSnapshot::new(Size { rows: 4, cols: 4 });
        g.cell_mut(3, 3).unwrap().text.push('Z');
        let buf = render(&g, 2, 2);
        assert_eq!(buf.area().width, 2);
        assert_eq!(buf.area().height, 2);
    }
}

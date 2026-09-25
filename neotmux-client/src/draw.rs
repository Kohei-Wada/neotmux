use anyhow::Context as _;
use neotmux_proto::{Color as PColor, GridSnapshot};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::{DefaultTerminal, Frame as TuiFrame};

use crate::overlay;
use neotmux_proto::screen::Frame;

/// Puts a frame on this process's terminal. The one place a produced frame meets a screen:
/// everything above it is what to show, and everything here is the showing.
pub fn show(terminal: &mut DefaultTerminal, frame: &Frame) -> anyhow::Result<()> {
    terminal
        .draw(|tui| {
            let area = tui.area();
            let mut focused_cell = None;
            for pane in &frame.panes {
                let cell = Rect {
                    x: area.x + pane.rect.x,
                    y: area.y + pane.rect.y,
                    width: pane.rect.w,
                    height: pane.rect.h,
                }
                .intersection(area);
                draw_grid(tui, cell, pane.grid);
                if frame.cursor == Some(pane.pane) {
                    focused_cell = Some((cell, pane.grid));
                }
            }
            if let Some(position) =
                focused_cell.and_then(|(cell, grid)| cursor_position(cell, grid))
            {
                tui.set_cursor_position(position);
            }
            // The whole area, not the pane area: a `pane = nil` command is in surface
            // coordinates, which is what lets a source draw in the band it reserved.
            overlay::apply(tui, area, frame.overlay, frame.rects);
        })
        .context("draw")?;
    Ok(())
}

pub(crate) fn color(c: PColor) -> Color {
    match c {
        PColor::Default => Color::Reset,
        PColor::Idx(i) => Color::Indexed(i),
        PColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// The one place the terminal's single cursor is put. It belongs in the focused pane, because
/// that is where the keys are going, at that pane's own cursor cell.
///
/// `None` parks it nowhere, which hides it, and that is the answer whenever the focused pane
/// has no visible cell to offer: its grid hides its own cursor, the cursor sits outside the
/// rect the pane was given, or — §6's case — the terminal shrank until the focused pane could
/// not be shown at all. The panes never overlap, so every other visible cell belongs to a pane
/// that is *not* receiving the keys, or to the gutter between two of them; a cursor blinking
/// in one of those would say the keys are going somewhere they are not. Typing blind is the documented cost of a
/// terminal too small for the tree; lying about where the typing lands is not.
pub fn cursor_position(area: Rect, grid: &GridSnapshot) -> Option<Position> {
    if grid.hide_cursor {
        return None;
    }
    let (row, col) = grid.cursor;
    (row < area.height && col < area.width).then(|| Position::new(area.x + col, area.y + row))
}

/// Draws one pane's grid into `area`.
pub fn draw_grid(frame: &mut TuiFrame, area: Rect, grid: &GridSnapshot) {
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
}

#[cfg(test)]
mod tests {
    use neotmux_proto::{Color as PColor, GridSnapshot, Size};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::{Position, Rect};
    use ratatui::style::Color;

    use super::{cursor_position, draw_grid};

    fn grid_at(row: u16, col: u16) -> GridSnapshot {
        let mut grid = GridSnapshot::new(Size { rows: 4, cols: 4 });
        grid.cursor = (row, col);
        grid
    }

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
    fn clips_to_the_area() {
        let mut g = GridSnapshot::new(Size { rows: 4, cols: 4 });
        g.cell_mut(3, 3).unwrap().text.push('Z');
        let buf = render(&g, 2, 2);
        assert_eq!(buf.area().width, 2);
        assert_eq!(buf.area().height, 2);
    }

    #[test]
    fn the_cursor_lands_on_the_focused_panes_own_cell() {
        let area = Rect::new(10, 5, 4, 4);
        assert_eq!(
            cursor_position(area, &grid_at(1, 2)),
            Some(Position::new(12, 6))
        );
    }

    #[test]
    fn a_grid_that_hides_its_cursor_parks_it_nowhere() {
        let mut grid = grid_at(1, 2);
        grid.hide_cursor = true;
        assert_eq!(cursor_position(Rect::new(0, 0, 4, 4), &grid), None);
    }

    #[test]
    fn a_cursor_outside_the_rect_the_pane_was_given_parks_nowhere() {
        assert_eq!(cursor_position(Rect::new(0, 0, 2, 2), &grid_at(3, 3)), None);
    }

    /// §6: the terminal shrank until the focused pane cannot be shown. No pane offers the
    /// cursor a cell, and the terminal ends the frame with it hidden rather than blinking in
    /// a pane the keys are not going to.
    #[test]
    fn a_frame_that_parks_the_cursor_nowhere_leaves_it_hidden() {
        let mut terminal = Terminal::new(TestBackend::new(4, 4)).expect("terminal");
        terminal
            .draw(|frame| frame.set_cursor_position(Position::new(1, 1)))
            .expect("draw");
        assert!(terminal.backend().cursor_visible());

        terminal.draw(|_| {}).expect("draw");
        assert!(
            !terminal.backend().cursor_visible(),
            "the cursor stayed visible with no pane to own it"
        );
    }
}

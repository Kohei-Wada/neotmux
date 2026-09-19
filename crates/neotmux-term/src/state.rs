use neotmux_core::proto::{Color, GridSnapshot, Size};

pub struct TermState {
    parser: vt100::Parser,
}

fn color_from_vt100(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(i) => Color::Idx(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

impl TermState {
    pub fn new(size: Size, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new(size.rows, size.cols, scrollback),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, size: Size) {
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }

    pub fn size(&self) -> Size {
        let (rows, cols) = self.parser.screen().size();
        Size { rows, cols }
    }

    pub fn snapshot_into(&self, out: &mut GridSnapshot) {
        let screen = self.parser.screen();
        let size = self.size();
        out.resize(size);
        out.cursor = screen.cursor_position();
        out.hide_cursor = screen.hide_cursor();
        for row in 0..size.rows {
            for col in 0..size.cols {
                let Some(src) = screen.cell(row, col) else {
                    continue;
                };
                let dst = out.cell_mut(row, col).expect("in bounds");
                dst.text.push_str(src.contents());
                dst.fg = color_from_vt100(src.fgcolor());
                dst.bg = color_from_vt100(src.bgcolor());
                dst.bold = src.bold();
                dst.italic = src.italic();
                dst.underline = src.underline();
                dst.inverse = src.inverse();
                dst.wide = src.is_wide();
                dst.wide_continuation = src.is_wide_continuation();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: &TermState) -> GridSnapshot {
        let mut out = GridSnapshot::default();
        state.snapshot_into(&mut out);
        out
    }

    #[test]
    fn plain_text_lands_in_the_grid() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.feed(b"hi");
        let g = snap(&s);
        assert_eq!(g.cell(0, 0).unwrap().text, "h");
        assert_eq!(g.cell(0, 1).unwrap().text, "i");
        assert_eq!(g.cursor, (0, 2));
    }

    #[test]
    fn sgr_bold_and_indexed_colour_are_carried() {
        let mut s = TermState::new(Size { rows: 1, cols: 4 }, 0);
        s.feed(b"\x1b[1;31mR\x1b[0m");
        let c = snap(&s);
        let cell = c.cell(0, 0).unwrap();
        assert_eq!(cell.text, "R");
        assert!(cell.bold);
        assert_eq!(cell.fg, Color::Idx(1));
    }

    #[test]
    fn truecolour_is_carried() {
        let mut s = TermState::new(Size { rows: 1, cols: 2 }, 0);
        s.feed(b"\x1b[38;2;10;20;30mZ");
        assert_eq!(snap(&s).cell(0, 0).unwrap().fg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn cursor_positioning_and_erase_work() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.feed(b"abcd\x1b[1;1H\x1b[K");
        let g = snap(&s);
        assert_eq!(g.cell(0, 0).unwrap().text, "");
        assert_eq!(g.cursor, (0, 0));
    }

    #[test]
    fn resize_is_reflected_in_the_snapshot() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.resize(Size { rows: 3, cols: 6 });
        assert_eq!(s.size(), Size { rows: 3, cols: 6 });
        assert_eq!(snap(&s).size, Size { rows: 3, cols: 6 });
    }

    #[test]
    fn snapshot_into_reuses_the_buffer() {
        let s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        let mut out = GridSnapshot::default();
        s.snapshot_into(&mut out);
        let ptr = out.cells.as_ptr();
        s.snapshot_into(&mut out);
        assert_eq!(out.cells.as_ptr(), ptr);
    }
}

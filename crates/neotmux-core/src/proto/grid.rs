use crate::proto::geom::Size;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Color {
    #[default]
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SnapCell {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
    pub wide_continuation: bool,
}

impl SnapCell {
    pub fn reset(&mut self) {
        self.text.clear();
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.bold = false;
        self.italic = false;
        self.underline = false;
        self.inverse = false;
        self.wide = false;
        self.wide_continuation = false;
    }
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct GridSnapshot {
    pub size: Size,
    pub cursor: (u16, u16),
    pub hide_cursor: bool,
    pub cells: Vec<SnapCell>,
}

impl GridSnapshot {
    pub fn new(size: Size) -> Self {
        let mut this = Self::default();
        this.resize(size);
        this
    }

    pub fn resize(&mut self, size: Size) {
        self.size = size;
        self.cells.resize(size.area(), SnapCell::default());
        for cell in &mut self.cells {
            cell.reset();
        }
    }

    fn index(&self, row: u16, col: u16) -> Option<usize> {
        if row >= self.size.rows || col >= self.size.cols {
            return None;
        }
        Some(usize::from(row) * usize::from(self.size.cols) + usize::from(col))
    }

    pub fn cell(&self, row: u16, col: u16) -> Option<&SnapCell> {
        self.index(row, col).map(|i| &self.cells[i])
    }

    pub fn cell_mut(&mut self, row: u16, col: u16) -> Option<&mut SnapCell> {
        self.index(row, col).map(|i| &mut self.cells[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_row_major() {
        let mut g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        g.cell_mut(1, 2).unwrap().text.push('X');
        assert_eq!(g.cells[5].text, "X");
        assert_eq!(g.cell(1, 2).unwrap().text, "X");
    }

    #[test]
    fn rejects_out_of_bounds() {
        let g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        assert!(g.cell(2, 0).is_none());
        assert!(g.cell(0, 3).is_none());
    }

    #[test]
    fn resize_to_same_size_keeps_the_allocation() {
        let mut g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        let ptr = g.cells.as_ptr();
        g.resize(Size { rows: 2, cols: 3 });
        assert_eq!(g.cells.as_ptr(), ptr);
    }

    #[test]
    fn resize_clears_cells() {
        let mut g = GridSnapshot::new(Size { rows: 1, cols: 1 });
        g.cell_mut(0, 0).unwrap().text.push('X');
        g.resize(Size { rows: 1, cols: 1 });
        assert_eq!(g.cell(0, 0).unwrap().text, "");
    }
}

use crate::geom::Size;
use crate::geom::SnapCell;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

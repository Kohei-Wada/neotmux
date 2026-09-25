use crate::geom::Size;

/// The smallest grid `vt100` survives arbitrary output on. Measured against vt100 0.16.2:
/// one row underflows in `grid::col_wrap` the first time a line wraps, and one column
/// underflows in `screen::write_grapheme` the first time a wide character is written.
pub const MIN_GRID: Size = Size { rows: 2, cols: 2 };

/// The one place a size is clamped before it reaches `vt100` or the PTY.
pub fn clamp_to_min_grid(size: Size) -> Size {
    Size {
        rows: size.rows.max(MIN_GRID.rows),
        cols: size.cols.max(MIN_GRID.cols),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::GridSnapshot;

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

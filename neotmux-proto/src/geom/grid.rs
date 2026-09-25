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

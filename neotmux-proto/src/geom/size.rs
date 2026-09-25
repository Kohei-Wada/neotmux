#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    pub fn area(self) -> usize {
        usize::from(self.rows) * usize::from(self.cols)
    }
}

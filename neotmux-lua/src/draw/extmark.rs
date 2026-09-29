/// A range of one row of a buffer, coloured by a highlight group. Columns, not bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extmark {
    pub id: u32,
    pub ns: u32,
    pub row: usize,
    pub col: usize,
    /// `None` is to the end of the line.
    pub end_col: Option<usize>,
    pub hl_group: Option<String>,
    /// The group's `bg` across the whole width of the window on this row, under the text.
    pub line_hl_group: Option<String>,
    pub bold: Option<bool>,
}

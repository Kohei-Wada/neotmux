/// What to do with the blank rest of a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trailing {
    /// Cut it, so an untouched row is `""`. What reading a line of text wants.
    #[default]
    Trim,
    /// Keep it, so every row is exactly as wide as the pane. What a rectangle wants: the
    /// interior spacing of a short line is where its columns are.
    Pad,
}

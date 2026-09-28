use crate::Color;

/// A stretch of one row whose cells all look the same: their text, and the colours and
/// attributes the program drew them in. What `neotmux.pane.content(id, { runs = true })`
/// answers per row, since a highlighter or a recolourer wants the stretches, not every cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Run {
    /// Whether `other` is drawn the same way, text aside: the test for one run carrying on.
    pub fn looks_like(&self, other: &Run) -> bool {
        self.fg == other.fg
            && self.bg == other.bg
            && self.bold == other.bold
            && self.dim == other.dim
            && self.italic == other.italic
            && self.underline == other.underline
            && self.inverse == other.inverse
    }
}

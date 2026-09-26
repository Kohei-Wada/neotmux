use crate::geom::Color;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SnapCell {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
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
        self.dim = false;
        self.italic = false;
        self.underline = false;
        self.inverse = false;
        self.wide = false;
        self.wide_continuation = false;
    }
}

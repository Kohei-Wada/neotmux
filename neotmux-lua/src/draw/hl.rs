use neotmux_proto::Color;

/// One highlight group: what it paints. A channel left `None` keeps what is under it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hl {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: Option<bool>,
}

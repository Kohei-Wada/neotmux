use crate::plugin::KindTag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Tick,
    Event(String),
    Key,
    Keymap(String),
    Write,
}

impl Kind {
    pub fn tag(&self) -> KindTag {
        match self {
            Kind::Tick => KindTag::Tick,
            Kind::Event(_) => KindTag::Event,
            Kind::Key => KindTag::Key,
            Kind::Keymap(_) => KindTag::Keymap,
            Kind::Write => KindTag::Write,
        }
    }
}

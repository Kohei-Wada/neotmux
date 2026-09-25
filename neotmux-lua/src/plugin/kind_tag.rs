#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindTag {
    Tick,
    Event,
    Key,
    Keymap,
    Write,
}

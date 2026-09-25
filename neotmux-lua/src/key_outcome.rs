#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    Consumed,
    Fallthrough,
}

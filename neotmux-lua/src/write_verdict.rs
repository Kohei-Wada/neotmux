/// What the handlers said about a write. `Refused` carries the sentence the caller is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteVerdict {
    Pass,
    Refused(String),
}

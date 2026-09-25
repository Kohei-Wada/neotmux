/// What `show` did, which decides whether anything is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Switched,
    AlreadyCurrent,
    NoSuchWindow,
}

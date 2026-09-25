#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFailure {
    pub source: String,
    pub message: String,
}

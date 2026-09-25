/// What `run_command` answered. The host caller — the CLI, in the spec's next half — tells
/// these apart to decide whether a built-in of the same name should run instead (spec §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Ran(Vec<String>),
    Failed(String),
    Unknown,
}

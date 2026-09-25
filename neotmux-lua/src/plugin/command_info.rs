use crate::plugin::SourceId;

/// One registered command, as `list()` reports it — the owner already resolved to nothing
/// more than an id, because naming it is `Sources`' job, not this one's.
#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: String,
    pub desc: Option<String>,
    pub owner: SourceId,
}

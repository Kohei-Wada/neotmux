/// One registered command, named the way `commands()` hands them to a host caller: the
/// owner already resolved to the display name `Sources` gives it, so the CLI never has to
/// reach back into the Lua layer to say where a command came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    pub name: String,
    pub desc: Option<String>,
    pub source: String,
}

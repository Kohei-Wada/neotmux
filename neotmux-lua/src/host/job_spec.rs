/// What to run. The command is a list because a shell is the caller's to ask for by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpec {
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    /// Added to whatever the host already has, never a replacement: a plugin cannot read the
    /// environment, so it has no way to compose a whole one.
    pub env: Vec<(String, String)>,
}

/// One process of a job: its `comm`, and its argv where reading it was safe.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JobProcess {
    pub pid: i32,
    pub name: String,
    pub argv: Option<Vec<String>>,
}

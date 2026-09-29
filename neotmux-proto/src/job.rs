use crate::JobProcess;

/// A pane's foreground job: its process group, and the members of it that could be read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Job {
    pub group: i32,
    pub processes: Vec<JobProcess>,
}

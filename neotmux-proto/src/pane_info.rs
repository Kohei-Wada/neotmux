/// What a pane is doing, as a status line reads it: the command its foreground process is
/// running and that process's working directory.
///
/// Both are optional because both can genuinely be unknown — a child that exited between the
/// host asking and the answer coming back leaves nothing to read. That is not an error, so it
/// is not modelled as one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub command: Option<String>,
    pub cwd: Option<String>,
    /// Every process of the foreground job, for telling what is running under a wrapper: a
    /// `node` handed Codex's script is `command = "node"`, and only its argv says Codex.
    pub job: Option<Job>,
}

/// A pane's foreground job: its process group, and the members of it that could be read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Job {
    pub group: i32,
    pub processes: Vec<JobProcess>,
}

/// One process of a job: its `comm`, and its argv where reading it was safe.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JobProcess {
    pub pid: i32,
    pub name: String,
    pub argv: Option<Vec<String>>,
}

impl PaneInfo {
    pub fn is_unknown(&self) -> bool {
        self.command.is_none() && self.cwd.is_none() && self.job.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_nothing_could_be_read_about_is_unknown() {
        assert!(PaneInfo::default().is_unknown());
    }

    #[test]
    fn one_fact_is_enough_to_not_be_unknown() {
        let info = PaneInfo {
            command: Some("bash".to_string()),
            cwd: None,
            job: None,
        };
        assert!(!info.is_unknown());
    }
}

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
}

impl PaneInfo {
    pub fn is_unknown(&self) -> bool {
        self.command.is_none() && self.cwd.is_none()
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
        };
        assert!(!info.is_unknown());
    }
}

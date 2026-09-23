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

/// Where a pane's cursor is and what its program has done to the terminal, published with
/// every frame.
///
/// Per frame rather than per question: the answer is a syscall and a plugin asks it every
/// tick, so the cost is paid once by the host instead of once per caller. The price is that
/// it describes the pane as of this frame, which is the same bargain `revisions` already
/// makes, and the one a plugin answering a keystroke can afford.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PaneTty {
    /// Where the program put the cursor, in the pane's own cells: row then column, the order
    /// the terminal itself counts in.
    pub cursor: (u16, u16),
    /// Whether the program is showing it. A hidden cursor is still somewhere.
    pub cursor_visible: bool,
    /// Whether the pane is on the alternate screen — a full-screen program is running.
    pub alternate: bool,
    /// Whether the program asked for application cursor keys (DECCKM), which is what an arrow
    /// key is encoded as when it is sent.
    pub application_cursor: bool,
    /// The slave's `ICANON` and `ECHO`, read from the master. `None` where the master could
    /// not be asked, which is not an error and must not be guessed at: a plugin that cannot
    /// tell a password prompt from a shell must do nothing rather than something.
    pub line: Option<LineMode>,
}

/// What the program has told the tty to do with a line as it is typed.
///
/// | canonical | echo | what it means |
/// | --- | --- | --- |
/// | on | on | a program with no line editor of its own is reading — a plugin may take over |
/// | on | off | a password prompt — never touch it |
/// | off | off | the program edits for itself (readline, vim, less) |
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LineMode {
    pub canonical: bool,
    pub echo: bool,
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

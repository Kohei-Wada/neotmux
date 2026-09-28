/// Which of OSC 133's four marks a shell sent: `A`, `B`, `C` or `D`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    /// A prompt starts here.
    Prompt,
    /// The command line the user types starts here.
    Input,
    /// The command's output starts here.
    Output,
    /// The command finished.
    Done,
}

impl MarkKind {
    /// The spelling a plugin reads.
    pub fn name(self) -> &'static str {
        match self {
            MarkKind::Prompt => "prompt",
            MarkKind::Input => "input",
            MarkKind::Output => "output",
            MarkKind::Done => "done",
        }
    }
}

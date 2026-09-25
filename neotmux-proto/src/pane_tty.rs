use crate::LineMode;

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

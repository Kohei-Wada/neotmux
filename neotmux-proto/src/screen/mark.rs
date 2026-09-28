use crate::screen::MarkKind;

/// One OSC 133 mark, kept against the line it landed on. `line` counts the way a pane's
/// history does — `dropped + index`, the history's lines first and then the screen's rows — so
/// it names the same line however far the screen has scrolled since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub line: u64,
    pub col: u16,
    pub kind: MarkKind,
    /// The exit status a `D` carried, when it carried one that is a number.
    pub exit: Option<i32>,
}

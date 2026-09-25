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

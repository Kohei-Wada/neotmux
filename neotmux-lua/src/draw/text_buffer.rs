use crate::draw::Extmark;

/// One buffer: its lines and what colours them. A new one is one empty line, as Neovim's is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBuffer {
    pub lines: Vec<String>,
    pub extmarks: Vec<Extmark>,
    pub next_extmark: u32,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            extmarks: Vec::new(),
            next_extmark: 1,
        }
    }
}

impl TextBuffer {
    /// Lines `start..end` become `lines`. An extmark on a replaced line is dropped — the one
    /// place this is not Neovim, where it would collapse to the start — and one below moves
    /// with its line. The caller has checked the range.
    pub fn set_lines(&mut self, start: usize, end: usize, lines: Vec<String>) {
        let added = lines.len();
        self.lines.splice(start..end, lines);
        let removed = end - start;
        self.extmarks
            .retain(|mark| mark.row < start || mark.row >= end);
        for mark in &mut self.extmarks {
            if mark.row >= end {
                mark.row = mark.row + added - removed;
            }
        }
        // Neovim keeps one empty line in a buffer with none.
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
    }
}

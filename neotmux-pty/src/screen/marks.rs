//! The OSC 133 marks a pane's shell has sent, in line order.

use neotmux_proto::screen::{Mark, MarkKind};

/// What a shell said about its own structure: where its prompts, command lines and outputs
/// start, and how its commands ended. Kept by the rules a redrawn prompt needs — a prompt
/// replaces whatever is below it, since that has been written over, and a mark replaces one of
/// its own kind on its line — and let go of with the lines they are on.
#[derive(Default)]
pub struct Marks {
    held: Vec<Mark>,
}

impl Marks {
    /// The kind and exit status of an OSC 133 payload, as `vt100` split it on `;`, or `None`
    /// for any other OSC.
    pub fn parse(params: &[&[u8]]) -> Option<(MarkKind, Option<i32>)> {
        let [b"133", kind, rest @ ..] = params else {
            return None;
        };
        let kind = match *kind {
            b"A" => MarkKind::Prompt,
            b"B" => MarkKind::Input,
            b"C" => MarkKind::Output,
            b"D" => MarkKind::Done,
            _ => return None,
        };
        let exit = match (kind, rest.first()) {
            (MarkKind::Done, Some(status)) => std::str::from_utf8(status)
                .ok()
                .and_then(|status| status.parse().ok()),
            _ => None,
        };
        Some((kind, exit))
    }

    pub fn add(&mut self, mark: Mark) {
        if mark.kind == MarkKind::Prompt {
            self.held
                .truncate(self.held.partition_point(|held| held.line <= mark.line));
        }
        let start = self.held.partition_point(|held| held.line < mark.line);
        let end = self.held.partition_point(|held| held.line <= mark.line);
        if let Some(i) = self.held[start..end]
            .iter()
            .position(|held| held.kind == mark.kind)
        {
            self.held.remove(start + i);
        }
        let at = self.held.partition_point(|held| held.line <= mark.line);
        self.held.insert(at, mark);
    }

    /// Lets go of the marks on lines before `first_held`, which the history no longer holds.
    pub fn prune(&mut self, first_held: u64) {
        let gone = self.held.partition_point(|held| held.line < first_held);
        self.held.drain(..gone);
    }

    pub fn between(&self, first: u64, last: u64) -> Vec<Mark> {
        let start = self.held.partition_point(|held| held.line < first);
        let end = self.held.partition_point(|held| held.line <= last);
        self.held[start..end].to_vec()
    }
}

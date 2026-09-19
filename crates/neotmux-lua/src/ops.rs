use std::cell::RefCell;

use neotmux_core::proto::{PaneId, SplitDir};

use crate::source::SourceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Quit,
    Notify { level: Level, text: String },
    Log { level: Level, text: String },
    PaneSplit { dir: SplitDir },
    PaneFocus { pane: PaneId },
}

#[derive(Default)]
pub struct OpBuffer {
    ops: RefCell<Vec<(SourceId, Op)>>,
}

impl OpBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, from: SourceId, op: Op) {
        self.ops.borrow_mut().push((from, op));
    }

    pub fn drain(&self) -> Vec<(SourceId, Op)> {
        std::mem::take(&mut *self.ops.borrow_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Sources;

    #[test]
    fn ops_come_back_in_order_with_their_owner() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let b = sources.intern("beta");
        let buf = OpBuffer::new();
        buf.push(a, Op::Quit);
        buf.push(
            b,
            Op::Notify {
                level: Level::Warn,
                text: "careful".to_string(),
            },
        );
        let drained = buf.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, a);
        assert_eq!(drained[0].1, Op::Quit);
        assert_eq!(drained[1].0, b);
    }

    #[test]
    fn draining_empties_the_buffer() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let buf = OpBuffer::new();
        buf.push(a, Op::Quit);
        assert_eq!(buf.drain().len(), 1);
        assert!(buf.drain().is_empty());
    }

    #[test]
    fn pushing_during_a_drain_lands_in_the_next_drain() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let buf = OpBuffer::new();
        buf.push(a, Op::Quit);
        let first = buf.drain();
        buf.push(
            a,
            Op::Log {
                level: Level::Info,
                text: "later".to_string(),
            },
        );
        assert_eq!(first.len(), 1);
        assert_eq!(buf.drain().len(), 1);
    }
}

use std::cell::RefCell;

use neotmux_proto::{Dir, Level, PaneId, SplitDir, WindowId};

use crate::source::SourceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Quit,
    Notify { level: Level, text: String },
    Log { level: Level, text: String },
    PaneSplit { dir: SplitDir },
    PaneFocus { pane: PaneId },
    PaneResize { dir: Dir, cells: u16 },
    WindowCreate { window: WindowId },
    WindowFocus { window: WindowId },
}

/// Where the buffer ended before a source's tick group ran, so that group's own ops can be
/// taken back out if its frame is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark(usize);

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

    pub fn mark(&self) -> Mark {
        Mark(self.ops.borrow().len())
    }

    /// Drops what `from` pushed after `mark`. Position alone would not do: a tick can push an
    /// op attributed to the library its callback was written in, and that source's ops are not
    /// this frame's to throw away.
    pub fn discard_from(&self, from: SourceId, mark: Mark) {
        let mut ops = self.ops.borrow_mut();
        let mut index = 0;
        ops.retain(|(owner, _)| {
            let keep = index < mark.0 || *owner != from;
            index += 1;
            keep
        });
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
    fn discarding_takes_back_one_source_and_leaves_the_rest() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let b = sources.intern("beta");
        let buf = OpBuffer::new();
        buf.push(a, Op::Quit);
        let mark = buf.mark();
        buf.push(
            a,
            Op::PaneSplit {
                dir: SplitDir::Horizontal,
            },
        );
        buf.push(b, Op::Quit);
        buf.push(
            a,
            Op::PaneFocus {
                pane: PaneId::from(1),
            },
        );

        buf.discard_from(a, mark);

        let left = buf.drain();
        assert_eq!(left.len(), 2, "{left:?}");
        assert_eq!(left[0], (a, Op::Quit), "an op pushed before the mark went");
        assert_eq!(left[1].0, b, "another source's op went with the discard");
    }

    #[test]
    fn an_op_pushed_after_a_discard_survives_it() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let buf = OpBuffer::new();
        let mark = buf.mark();
        buf.push(a, Op::Quit);
        buf.discard_from(a, mark);
        buf.push(
            a,
            Op::Log {
                level: Level::Error,
                text: "overran".to_string(),
            },
        );

        assert_eq!(buf.drain().len(), 1);
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

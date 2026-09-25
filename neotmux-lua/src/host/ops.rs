use std::cell::RefCell;
use std::rc::Rc;

use neotmux_proto::{Dir, Level, PaneId, SessionId, SplitDir, WindowId};

use crate::host::Ambient;
use crate::plugin::SourceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Quit,
    Detach,
    Notify {
        level: Level,
        text: String,
    },
    Log {
        level: Level,
        text: String,
    },
    PaneSplit {
        dir: SplitDir,
    },
    PaneFocus {
        pane: PaneId,
    },
    PaneResize {
        dir: Dir,
        cells: u16,
    },
    PaneSend {
        pane: PaneId,
        text: String,
    },
    /// A keystroke, named the way `on_key` names one. The bytes it becomes are the host's to
    /// work out: they depend on what the pane has asked for, which is a fact about the pane
    /// rather than about the plugin relaying the key.
    PaneKey {
        pane: PaneId,
        spec: String,
    },
    WindowCreate {
        window: WindowId,
    },
    WindowFocus {
        window: WindowId,
    },
    /// A window's name, or `None` to take it away and leave it a number.
    WindowRename {
        window: WindowId,
        name: Option<String>,
    },
    /// The set of sessions changing, which is the server's rather than a session's: these
    /// three never reach `Session::apply`.
    SessionCreate {
        name: String,
    },
    SessionSwitch {
        name: String,
    },
    SessionKill {
        name: String,
    },
    SessionRename {
        from: String,
        to: String,
    },
    /// Take down every notification showing. The surface is the host's — it holds the ones
    /// still inside their time and the transcript of every one ever raised — so clearing it
    /// is a request like any other.
    NoticesClear,
}

/// Where the buffer ended before a source's tick group ran, so that group's own ops can be
/// taken back out if its frame is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark(usize);

pub struct OpBuffer {
    ops: RefCell<Vec<(Option<SessionId>, SourceId, Op)>>,
    /// Read at push time rather than passed in: an op is raised from a dozen places in `api`,
    /// and not one of them knows which session it is serving, nor should it.
    ambient: Rc<Ambient>,
}

impl OpBuffer {
    pub fn new(ambient: Rc<Ambient>) -> Self {
        Self {
            ops: RefCell::new(Vec::new()),
            ambient,
        }
    }

    pub fn push(&self, from: SourceId, op: Op) {
        self.ops
            .borrow_mut()
            .push((self.ambient.current(), from, op));
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
        ops.retain(|(_, owner, _)| {
            let keep = index < mark.0 || *owner != from;
            index += 1;
            keep
        });
    }

    /// What was raised for this session, taken out and nothing else. `None` is the load path:
    /// an op raised before any session existed belongs to no session, and `drain_load_ops` is
    /// the one caller that asks for those.
    pub fn drain_for(&self, session: Option<SessionId>) -> Vec<(SourceId, Op)> {
        let mut ops = self.ops.borrow_mut();
        let mut mine = Vec::new();
        ops.retain(|(raised, owner, op)| {
            if *raised == session {
                mine.push((*owner, op.clone()));
                false
            } else {
                true
            }
        });
        mine
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Sources;
    use neotmux_proto::Geometry;
    use std::cell::RefCell;

    /// A buffer with nothing bound: every op in these tests is raised outside a session, so
    /// `None` is what they are filed under and what they are drained by.
    fn buffer() -> OpBuffer {
        OpBuffer::new(Rc::new(Ambient::new(
            Rc::new(RefCell::new(Geometry::default())),
            Rc::new(RefCell::new(Default::default())),
            Rc::new(RefCell::new(Default::default())),
        )))
    }

    #[test]
    fn ops_come_back_in_order_with_their_owner() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let b = sources.intern("beta");
        let buf = buffer();
        buf.push(a, Op::Quit);
        buf.push(
            b,
            Op::Notify {
                level: Level::Warn,
                text: "careful".to_string(),
            },
        );
        let drained = buf.drain_for(None);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, a);
        assert_eq!(drained[0].1, Op::Quit);
        assert_eq!(drained[1].0, b);
    }

    #[test]
    fn draining_empties_the_buffer() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let buf = buffer();
        buf.push(a, Op::Quit);
        assert_eq!(buf.drain_for(None).len(), 1);
        assert!(buf.drain_for(None).is_empty());
    }

    #[test]
    fn discarding_takes_back_one_source_and_leaves_the_rest() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let b = sources.intern("beta");
        let buf = buffer();
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

        let left = buf.drain_for(None);
        assert_eq!(left.len(), 2, "{left:?}");
        assert_eq!(left[0], (a, Op::Quit), "an op pushed before the mark went");
        assert_eq!(left[1].0, b, "another source's op went with the discard");
    }

    #[test]
    fn an_op_pushed_after_a_discard_survives_it() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let buf = buffer();
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

        assert_eq!(buf.drain_for(None).len(), 1);
    }

    #[test]
    fn pushing_during_a_drain_lands_in_the_next_drain() {
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let buf = buffer();
        buf.push(a, Op::Quit);
        let first = buf.drain_for(None);
        buf.push(
            a,
            Op::Log {
                level: Level::Info,
                text: "later".to_string(),
            },
        );
        assert_eq!(first.len(), 1);
        assert_eq!(buf.drain_for(None).len(), 1);
    }
}

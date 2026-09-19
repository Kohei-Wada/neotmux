use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_core::proto::{DrawCmd, Epoch, StatusFragment};

use crate::source::SourceId;

#[derive(Debug, Clone, Default)]
pub struct Committed {
    pub epoch: Epoch,
    pub cmds: Vec<DrawCmd>,
    pub status: Vec<StatusFragment>,
}

#[derive(Default)]
struct Inner {
    epoch: Epoch,
    scratch: BTreeMap<SourceId, Committed>,
    committed: BTreeMap<SourceId, Committed>,
}

#[derive(Default)]
pub struct Buffers {
    inner: RefCell<Inner>,
}

impl Buffers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a frame under `epoch`. That epoch is the frame's, not the session's: it is what
    /// a commit is stamped with and what `applicable` measures against, so a geometry change
    /// after this call cannot invalidate what this frame already committed.
    pub fn begin(&self, epoch: Epoch) {
        let mut inner = self.inner.borrow_mut();
        inner.epoch = epoch;
        inner.scratch.clear();
    }

    pub fn record(&self, owner: SourceId, cmd: DrawCmd) {
        let mut inner = self.inner.borrow_mut();
        inner.scratch.entry(owner).or_default().cmds.push(cmd);
    }

    pub fn status(&self, owner: SourceId, fragment: StatusFragment) {
        let mut inner = self.inner.borrow_mut();
        inner
            .scratch
            .entry(owner)
            .or_default()
            .status
            .push(fragment);
    }

    pub fn commit(&self, owner: SourceId) {
        let mut inner = self.inner.borrow_mut();
        let epoch = inner.epoch.clone();
        // A missing scratch entry means the source recorded nothing this tick, not that
        // the tick failed; commit still replaces the previous frame with this empty one.
        // That also covers a source forgotten mid-tick: re-inserting an empty entry and
        // skipping it are indistinguishable to `applicable`, so the cheaper one wins.
        let mut entry = inner.scratch.remove(&owner).unwrap_or_default();
        entry.epoch = epoch;
        inner.committed.insert(owner, entry);
    }

    pub fn discard(&self, owner: SourceId) {
        self.inner.borrow_mut().scratch.remove(&owner);
    }

    pub fn forget(&self, owner: SourceId) {
        let mut inner = self.inner.borrow_mut();
        inner.scratch.remove(&owner);
        inner.committed.remove(&owner);
    }

    /// Everything committed under the frame this `begin` opened. A buffer left over from an
    /// earlier frame is replayed only while its epoch still matches.
    pub fn applicable(&self) -> (Vec<DrawCmd>, Vec<StatusFragment>) {
        let inner = self.inner.borrow();
        let mut cmds = Vec::new();
        let mut status = Vec::new();
        for entry in inner.committed.values() {
            if !entry.epoch.matches(&inner.epoch) {
                continue;
            }
            cmds.extend(entry.cmds.iter().cloned());
            status.extend(entry.status.iter().cloned());
        }
        (cmds, status)
    }
}

#[cfg(test)]
mod tests {
    use neotmux_core::proto::{Color, DrawCmd, Epoch, Size, StatusFragment};

    use super::*;
    use crate::source::Sources;

    fn epoch(rows: u16) -> Epoch {
        Epoch::new(Size { rows, cols: 80 }, vec![], None)
    }

    fn text(s: &str) -> DrawCmd {
        DrawCmd::Text {
            pane: None,
            x: 0,
            y: 0,
            text: s.to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }
    }

    fn fragment(s: &str) -> StatusFragment {
        StatusFragment::new(s, Color::Default, Color::Default)
    }

    #[test]
    fn a_committed_buffer_is_applicable_under_the_same_epoch() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.status(a, fragment("st"));
        buffers.commit(a);
        let (cmds, status) = buffers.applicable();
        assert_eq!(cmds, vec![text("hi")]);
        assert_eq!(status, vec![fragment("st")]);
    }

    #[test]
    fn a_stale_buffer_is_not_applicable() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.commit(a);
        buffers.begin(epoch(25));
        let (cmds, status) = buffers.applicable();
        assert!(cmds.is_empty());
        assert!(status.is_empty());
    }

    #[test]
    fn discard_leaves_the_previous_buffer_intact() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();

        buffers.begin(epoch(24));
        buffers.record(a, text("good"));
        buffers.commit(a);

        buffers.begin(epoch(24));
        buffers.record(a, text("half-written"));
        buffers.discard(a);

        let (cmds, _) = buffers.applicable();
        assert_eq!(cmds, vec![text("good")]);
    }

    #[test]
    fn a_commit_replaces_rather_than_appends() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("first"));
        buffers.commit(a);
        buffers.begin(epoch(24));
        buffers.record(a, text("second"));
        buffers.commit(a);
        let (cmds, _) = buffers.applicable();
        assert_eq!(cmds, vec![text("second")]);
    }

    #[test]
    fn nothing_recorded_means_nothing_applicable() {
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        let (cmds, status) = buffers.applicable();
        assert!(cmds.is_empty());
        assert!(status.is_empty());
    }

    #[test]
    fn sources_are_applied_in_id_order() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let b = sources.intern("b");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(b, text("second"));
        buffers.record(a, text("first"));
        buffers.commit(b);
        buffers.commit(a);
        let (cmds, _) = buffers.applicable();
        assert_eq!(cmds, vec![text("first"), text("second")]);
    }

    #[test]
    fn forget_drops_a_sources_buffer() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.commit(a);
        buffers.forget(a);
        let (cmds, _) = buffers.applicable();
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_commit_after_a_forget_contributes_nothing() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();

        buffers.begin(epoch(24));
        buffers.record(a, text("last words"));
        buffers.forget(a);
        buffers.commit(a);

        let (cmds, status) = buffers.applicable();
        assert!(cmds.is_empty());
        assert!(status.is_empty());
    }

    #[test]
    fn a_tick_that_commits_nothing_clears_the_previous_frame() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();

        buffers.begin(epoch(24));
        buffers.record(a, text("first"));
        buffers.commit(a);

        buffers.begin(epoch(24));
        buffers.commit(a);

        let (cmds, _) = buffers.applicable();
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_tick_that_raises_leaves_the_previous_frame_in_place() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();

        buffers.begin(epoch(24));
        buffers.record(a, text("first"));
        buffers.commit(a);

        buffers.begin(epoch(24));
        buffers.discard(a);

        let (cmds, _) = buffers.applicable();
        assert_eq!(cmds, vec![text("first")]);
    }

    #[test]
    fn a_scratch_left_uncommitted_does_not_leak_into_the_next_tick() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("orphan"));
        buffers.begin(epoch(24));
        buffers.record(a, text("fresh"));
        buffers.commit(a);
        let (cmds, _) = buffers.applicable();
        assert_eq!(cmds, vec![text("fresh")]);
    }
}

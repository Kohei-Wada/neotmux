use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_proto::{DrawCmd, Epoch, PaneId};

use crate::draw::FloatId;
use crate::plugin::SourceId;

#[derive(Debug, Clone, Default)]
pub struct Committed {
    pub epoch: Epoch,
    pub cmds: Vec<DrawCmd>,
}

/// One source's float scratch/commit: every float it drew this tick, in recording
/// order. Named so `Inner`'s maps don't spell out the nested tuple-of-vec-of-tuple
/// shape at every use (and so clippy's `type_complexity` lint has nothing to flag).
type FloatEntries = Vec<(FloatId, Vec<DrawCmd>)>;

#[derive(Default)]
struct Inner {
    epoch: Epoch,
    scratch: BTreeMap<SourceId, Committed>,
    committed: BTreeMap<SourceId, Committed>,
    scratch_f: BTreeMap<SourceId, FloatEntries>,
    committed_f: BTreeMap<SourceId, (Epoch, FloatEntries)>,
    /// Which panes a source is showing at a place in their own history, and how far back.
    /// Committed and discarded with that source's drawing, because it is part of the same
    /// picture: a frame thrown away must not leave a pane scrolled.
    scratch_v: BTreeMap<SourceId, BTreeMap<PaneId, usize>>,
    committed_v: BTreeMap<SourceId, (Epoch, BTreeMap<PaneId, usize>)>,
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
        inner.scratch_f.clear();
    }

    pub fn record(&self, owner: SourceId, cmd: DrawCmd) {
        let mut inner = self.inner.borrow_mut();
        inner.scratch.entry(owner).or_default().cmds.push(cmd);
    }

    /// Only a live scratch is visible to its owner for the rest of the tick, same as
    /// `record` — floats just live in a separate, per-owner ordered list because their
    /// assembly order is by float id across all owners, not by source.
    pub fn record_float(&self, owner: SourceId, id: FloatId, cmds: Vec<DrawCmd>) {
        let mut inner = self.inner.borrow_mut();
        inner.scratch_f.entry(owner).or_default().push((id, cmds));
    }

    /// This source asks for `pane` to be drawn `back` lines into its own history for this
    /// frame. The last word wins, the way a draw command's does.
    pub fn record_view(&self, owner: SourceId, pane: PaneId, back: usize) {
        self.inner
            .borrow_mut()
            .scratch_v
            .entry(owner)
            .or_default()
            .insert(pane, back);
    }

    pub fn commit(&self, owner: SourceId) {
        let mut inner = self.inner.borrow_mut();
        let epoch = inner.epoch.clone();
        // A missing scratch entry means the source recorded nothing this tick, not that
        // the tick failed; commit still replaces the previous frame with this empty one.
        // That also covers a source forgotten mid-tick: re-inserting an empty entry and
        // skipping it are indistinguishable to `applicable`, so the cheaper one wins.
        let mut entry = inner.scratch.remove(&owner).unwrap_or_default();
        entry.epoch = epoch.clone();
        inner.committed.insert(owner, entry);

        let floats = inner.scratch_f.remove(&owner).unwrap_or_default();
        inner.committed_f.insert(owner, (epoch.clone(), floats));

        let views = inner.scratch_v.remove(&owner).unwrap_or_default();
        inner.committed_v.insert(owner, (epoch, views));
    }

    pub fn discard(&self, owner: SourceId) {
        let mut inner = self.inner.borrow_mut();
        inner.scratch.remove(&owner);
        inner.scratch_f.remove(&owner);
        inner.scratch_v.remove(&owner);
    }

    pub fn forget(&self, owner: SourceId) {
        let mut inner = self.inner.borrow_mut();
        inner.scratch.remove(&owner);
        inner.committed.remove(&owner);
        inner.scratch_f.remove(&owner);
        inner.committed_f.remove(&owner);
        inner.scratch_v.remove(&owner);
        inner.committed_v.remove(&owner);
    }

    /// Every pane some source is showing at a place in its history, under the frame this
    /// `begin` opened. Two sources asking for the same pane is the later one winning, which
    /// is the rule the overlay already follows for the same cells.
    pub fn applicable_views(&self) -> BTreeMap<PaneId, usize> {
        let inner = self.inner.borrow();
        let mut out = BTreeMap::new();
        for (epoch, views) in inner.committed_v.values() {
            if *epoch == inner.epoch {
                out.extend(views.iter().map(|(pane, back)| (*pane, *back)));
            }
        }
        out
    }

    /// Everything committed under the frame this `begin` opened. A buffer left over from an
    /// earlier frame is replayed only while its epoch still matches.
    pub fn applicable(&self) -> Vec<DrawCmd> {
        let inner = self.inner.borrow();
        let mut cmds = Vec::new();
        for entry in inner.committed.values() {
            if !entry.epoch.matches(&inner.epoch) {
                continue;
            }
            cmds.extend(entry.cmds.iter().cloned());
        }
        cmds
    }

    /// Every float committed under the frame this `begin` opened, from any owner, sorted
    /// by `FloatId` so the caller extends in grant order. Same epoch discipline as
    /// `applicable`: a buffer left over from an earlier frame is replayed only while its
    /// epoch still matches. A float drawn into by more than one call in the same tick
    /// (one `ctx:text`, another `ctx:rect`, both naming the same handle) keeps every
    /// call's output, in recording order — the same union `applicable` gives a source's
    /// own commands, just keyed by float id instead of by owner.
    pub fn applicable_floats(&self) -> BTreeMap<FloatId, Vec<DrawCmd>> {
        let inner = self.inner.borrow();
        let mut floats: BTreeMap<FloatId, Vec<DrawCmd>> = BTreeMap::new();
        for (epoch, entries) in inner.committed_f.values() {
            if !epoch.matches(&inner.epoch) {
                continue;
            }
            for (id, cmds) in entries {
                floats.entry(*id).or_default().extend(cmds.iter().cloned());
            }
        }
        floats
    }
}

#[cfg(test)]
mod tests {
    use neotmux_proto::{Color, DrawCmd, Epoch, Size};

    use super::*;
    use crate::plugin::Sources;

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

    #[test]
    fn a_committed_buffer_is_applicable_under_the_same_epoch() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.commit(a);
        assert_eq!(buffers.applicable(), vec![text("hi")]);
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
        assert!(buffers.applicable().is_empty());
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

        let cmds = buffers.applicable();
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
        let cmds = buffers.applicable();
        assert_eq!(cmds, vec![text("second")]);
    }

    #[test]
    fn nothing_recorded_means_nothing_applicable() {
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        assert!(buffers.applicable().is_empty());
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
        let cmds = buffers.applicable();
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
        let cmds = buffers.applicable();
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

        assert!(buffers.applicable().is_empty());
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

        let cmds = buffers.applicable();
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

        let cmds = buffers.applicable();
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
        let cmds = buffers.applicable();
        assert_eq!(cmds, vec![text("fresh")]);
    }

    #[test]
    fn a_committed_float_is_applicable_under_the_same_epoch() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record_float(a, FloatId(1), vec![text("hi")]);
        buffers.commit(a);
        let mut expected = BTreeMap::new();
        expected.insert(FloatId(1), vec![text("hi")]);
        assert_eq!(buffers.applicable_floats(), expected);
    }

    #[test]
    fn a_float_recorded_on_a_discarded_frame_does_not_appear() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();

        buffers.begin(epoch(24));
        buffers.record_float(a, FloatId(1), vec![text("good")]);
        buffers.commit(a);

        buffers.begin(epoch(24));
        buffers.record_float(a, FloatId(1), vec![text("half-written")]);
        buffers.discard(a);

        let mut expected = BTreeMap::new();
        expected.insert(FloatId(1), vec![text("good")]);
        assert_eq!(buffers.applicable_floats(), expected);
    }

    #[test]
    fn committed_float_output_stops_replaying_once_the_epoch_changes() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();

        buffers.begin(epoch(24));
        buffers.record_float(a, FloatId(1), vec![text("hi")]);
        buffers.commit(a);

        let mut expected = BTreeMap::new();
        expected.insert(FloatId(1), vec![text("hi")]);
        assert_eq!(buffers.applicable_floats(), expected);

        buffers.begin(epoch(25));
        assert!(buffers.applicable_floats().is_empty());
    }

    #[test]
    fn forget_drops_both_the_surface_and_float_buffers() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.record_float(a, FloatId(1), vec![text("float")]);
        buffers.commit(a);
        buffers.forget(a);
        assert!(buffers.applicable().is_empty());
        assert!(buffers.applicable_floats().is_empty());
    }

    #[test]
    fn floats_are_returned_sorted_by_float_id_regardless_of_recording_order() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let b = sources.intern("b");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record_float(b, FloatId(2), vec![text("b's float")]);
        buffers.record_float(a, FloatId(1), vec![text("a's float")]);
        buffers.commit(b);
        buffers.commit(a);
        let ids: Vec<FloatId> = buffers.applicable_floats().into_keys().collect();
        assert_eq!(ids, vec![FloatId(1), FloatId(2)]);
    }

    #[test]
    fn multiple_draws_into_the_same_float_in_one_tick_all_survive_in_order() {
        let sources = Sources::new();
        let a = sources.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record_float(a, FloatId(1), vec![text("first")]);
        buffers.record_float(a, FloatId(1), vec![text("second")]);
        buffers.commit(a);
        let mut expected = BTreeMap::new();
        expected.insert(FloatId(1), vec![text("first"), text("second")]);
        assert_eq!(
            buffers.applicable_floats(),
            expected,
            "a second draw into the same float this tick should add to the first, not replace it"
        );
    }
}

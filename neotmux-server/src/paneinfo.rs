use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use neotmux_proto::{PaneId, PaneInfo};

/// How stale a pane's facts are allowed to get. A status plugin asks for them every tick, and
/// answering each ask from `/proc` would be three syscalls per pane per frame — sixty times a
/// second for something that changes when a command starts or a `cd` runs. Neither is
/// something a hand does more than a few times a second, and a quarter of a second is below
/// what reads as lag in a status bar.
const MAX_AGE: Duration = Duration::from_millis(250);

/// What every pane is doing, refreshed on a cadence rather than on demand.
///
/// The map is rebuilt whole on each refresh from whatever panes are live, so a pane that has
/// gone leaves no entry behind. Between refreshes the answer is the last one taken.
pub struct PaneInfoCache {
    info: BTreeMap<PaneId, PaneInfo>,
    /// `None` until the first refresh, and reset to it whenever the pane set changes, so a
    /// pane that has just opened or closed does not wait out the cadence.
    taken: Option<Instant>,
}

impl PaneInfoCache {
    pub fn new() -> Self {
        Self {
            info: BTreeMap::new(),
            taken: None,
        }
    }

    /// The pane set changed, so the next frame reads `/proc` whatever the cadence says.
    pub fn invalidate(&mut self) {
        self.taken = None;
    }

    fn is_due(&self, now: Instant) -> bool {
        match self.taken {
            None => true,
            Some(taken) => now.duration_since(taken) >= MAX_AGE,
        }
    }

    /// Rereads every pane's facts if the cadence says it is time, and reports whether it did.
    /// `read` is given each pane in turn and answers for it; it is a parameter so the cadence
    /// can be tested without a process to look at.
    pub fn refresh(
        &mut self,
        now: Instant,
        panes: impl Iterator<Item = PaneId>,
        mut read: impl FnMut(PaneId) -> PaneInfo,
    ) -> bool {
        if !self.is_due(now) {
            return false;
        }
        self.info = panes.map(|pane| (pane, read(pane))).collect();
        self.taken = Some(now);
        true
    }

    pub fn snapshot(&self) -> BTreeMap<PaneId, PaneInfo> {
        self.info.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u32) -> PaneId {
        PaneId::from(id)
    }

    fn named(name: &str) -> PaneInfo {
        PaneInfo {
            command: Some(name.to_string()),
            cwd: None,
        }
    }

    #[test]
    fn the_first_refresh_reads_and_the_next_one_in_the_same_moment_does_not() {
        let now = Instant::now();
        let mut cache = PaneInfoCache::new();
        let mut reads = 0;

        assert!(cache.refresh(now, [pane(1)].into_iter(), |_| {
            reads += 1;
            named("bash")
        }));
        assert!(!cache.refresh(now, [pane(1)].into_iter(), |_| {
            reads += 1;
            named("bash")
        }));

        assert_eq!(reads, 1);
        assert_eq!(cache.snapshot()[&pane(1)], named("bash"));
    }

    #[test]
    fn a_refresh_past_the_cadence_reads_again() {
        let now = Instant::now();
        let mut cache = PaneInfoCache::new();
        cache.refresh(now, [pane(1)].into_iter(), |_| named("bash"));

        assert!(cache.refresh(now + MAX_AGE, [pane(1)].into_iter(), |_| named("nvim")));
        assert_eq!(cache.snapshot()[&pane(1)], named("nvim"));
    }

    #[test]
    fn invalidating_makes_the_next_refresh_read_whatever_the_cadence_says() {
        let now = Instant::now();
        let mut cache = PaneInfoCache::new();
        cache.refresh(now, [pane(1)].into_iter(), |_| named("bash"));
        cache.invalidate();

        assert!(cache.refresh(now, [pane(1)].into_iter(), |_| named("nvim")));
        assert_eq!(cache.snapshot()[&pane(1)], named("nvim"));
    }

    #[test]
    fn a_pane_that_is_gone_leaves_no_entry_behind() {
        let now = Instant::now();
        let mut cache = PaneInfoCache::new();
        cache.refresh(now, [pane(1), pane(2)].into_iter(), |_| named("bash"));

        cache.invalidate();
        cache.refresh(now, [pane(1)].into_iter(), |_| named("bash"));

        assert!(!cache.snapshot().contains_key(&pane(2)));
    }
}

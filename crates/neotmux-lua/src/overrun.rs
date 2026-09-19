use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use mlua::RegistryKey;

use crate::api::forget_if_no_longer_drawing;
use crate::budget::Outcome;
use crate::draw::Buffers;
use crate::registry::{Kind, KindTag, RegistrationId, Registry};
use crate::source::SourceId;

/// What repeated overruns cost a source, and how that is undone. Measuring a tick is
/// `crate::budget`; this only counts and acts.
///
/// A parked registration keeps its callback rather than its registry entry: restoring it is
/// then the same `Registry::add` a fresh `neotmux.on_tick` performs, so there is one code path
/// that puts a tick subscription into the registry. Marking entries inactive instead would put
/// an "is it live?" question into `snapshot`, `owner`, `callback` and `len` — four places that
/// would each have to keep answering it correctly. The price is that the Lua handle held from
/// before the removal names an id that no longer exists; `handle:remove()` is idempotent, so
/// that is a no-op rather than a surprise.
/// How far past its budget an uninterrupted tick has to be before it is parked for it. The
/// elapsed figure is wall clock across the whole group — host work, a GC step another source
/// triggered, the process being descheduled — so one budget's worth of it is jitter.
const RAN_LONG_PARK_FACTOR: u32 = 2;

/// Whether an overrun is severe enough to be held against the source. Every overrun is still
/// reported: what this softens is parking, not attribution.
pub fn counts_toward_park(outcome: Outcome, budget: Duration) -> bool {
    match outcome {
        Outcome::WithinBudget => false,
        // The hook watched the deadline pass, so there is nothing inferred about this one.
        Outcome::Interrupted => true,
        Outcome::RanLong { elapsed } => elapsed > budget * RAN_LONG_PARK_FACTOR,
    }
}

#[derive(Default)]
pub struct Overrun {
    consecutive: RefCell<BTreeMap<SourceId, u32>>,
    parked: RefCell<BTreeMap<SourceId, Vec<Rc<RegistryKey>>>>,
}

impl Overrun {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one overrun. True when it is the one that reaches the limit.
    pub fn count(&self, source: SourceId, limit: u32) -> bool {
        let mut consecutive = self.consecutive.borrow_mut();
        let seen = consecutive.entry(source).or_insert(0);
        *seen += 1;
        *seen >= limit
    }

    /// A tick that stayed inside its budget wipes the source's slate (§5.3).
    pub fn forgive(&self, source: SourceId) {
        self.consecutive.borrow_mut().remove(&source);
    }

    /// Removes the source's tick subscriptions, keeping their callbacks for `restore`.
    pub fn park(&self, registry: &Registry, buffers: &Buffers, source: SourceId) {
        let mut callbacks = Vec::new();
        for id in registry.snapshot(KindTag::Tick) {
            if registry.owner(id) != Some(source) {
                continue;
            }
            if let Some(callback) = registry.callback(id) {
                callbacks.push(callback);
            }
            registry.remove(id);
        }
        forget_if_no_longer_drawing(registry, buffers, source);
        if !callbacks.is_empty() {
            self.parked
                .borrow_mut()
                .entry(source)
                .or_default()
                .extend(callbacks);
        }
        self.forgive(source);
    }

    /// Drops what a source has parked, without restoring it. A source that registers a tick
    /// again has replaced what was taken from it — a re-`require`d chunk is the usual way —
    /// and keeping the old callbacks would let a later `restore` put stale duplicates back
    /// beside the fresh registration.
    pub fn forget_parked(&self, source: SourceId) {
        self.parked.borrow_mut().remove(&source);
    }

    /// Puts them back, with a clean counter so a restored source gets a full limit's worth of
    /// frames again. The new registrations are returned because they carry fresh ids: a Lua
    /// handle held from before the park names an id that no longer exists, so without these
    /// the restored subscription could never be removed again. Empty when this source has
    /// nothing parked.
    pub fn restore(&self, registry: &Registry, source: SourceId) -> Vec<RegistrationId> {
        let Some(callbacks) = self.parked.borrow_mut().remove(&source) else {
            return Vec::new();
        };
        let restored = callbacks
            .into_iter()
            .map(|callback| registry.add(source, Kind::Tick, callback))
            .collect();
        self.forgive(source);
        restored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: Duration = Duration::from_millis(5);

    #[test]
    fn an_interrupted_overrun_counts_at_the_first_offence() {
        assert!(counts_toward_park(Outcome::Interrupted, BUDGET));
    }

    #[test]
    fn a_tick_inside_its_budget_counts_for_nothing() {
        assert!(!counts_toward_park(Outcome::WithinBudget, BUDGET));
    }

    #[test]
    fn an_uninterrupted_overrun_within_jitter_is_reported_but_not_counted() {
        let outcome = Outcome::RanLong {
            elapsed: Duration::from_millis(9),
        };
        assert!(!counts_toward_park(outcome, BUDGET));
    }

    #[test]
    fn an_uninterrupted_overrun_past_twice_the_budget_is_counted() {
        let outcome = Outcome::RanLong {
            elapsed: Duration::from_millis(11),
        };
        assert!(counts_toward_park(outcome, BUDGET));
    }
}

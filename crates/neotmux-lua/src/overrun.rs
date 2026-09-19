use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::RegistryKey;

use crate::api::forget_if_no_longer_drawing;
use crate::draw::Buffers;
use crate::registry::{Kind, KindTag, Registry};
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

    /// Puts them back, with a clean counter so a restored source gets a full limit's worth of
    /// frames again. False when this source has nothing parked.
    pub fn restore(&self, registry: &Registry, source: SourceId) -> bool {
        let Some(callbacks) = self.parked.borrow_mut().remove(&source) else {
            return false;
        };
        for callback in callbacks {
            registry.add(source, Kind::Tick, callback);
        }
        self.forgive(source);
        true
    }
}

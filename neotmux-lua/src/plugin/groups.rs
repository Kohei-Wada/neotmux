use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use crate::plugin::{GroupId, Held, InGroup};

/// Every group there is, what each holds, and which one a `g.*` call is running for.
#[derive(Default)]
pub struct Groups {
    next: Cell<u64>,
    by_name: RefCell<BTreeMap<String, GroupId>>,
    held: RefCell<BTreeMap<GroupId, Vec<Held>>>,
    pub(crate) current: Cell<Option<GroupId>>,
    /// The groups named since `begin_reload`, while one is in progress.
    named: RefCell<Option<BTreeSet<GroupId>>>,
}

impl Groups {
    /// The group of that name, made if there is none, and what it held — which the caller
    /// takes back, since naming a group is emptying it.
    pub fn name(&self, name: &str) -> (GroupId, Vec<Held>) {
        let id = *self
            .by_name
            .borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| {
                let id = GroupId(self.next.get() + 1);
                self.next.set(id.0);
                id
            });
        if let Some(named) = self.named.borrow_mut().as_mut() {
            named.insert(id);
        }
        let held = self.held.borrow_mut().remove(&id).unwrap_or_default();
        (id, held)
    }

    pub fn enter(&self, id: GroupId) -> InGroup<'_> {
        let was = self.current.replace(Some(id));
        InGroup { groups: self, was }
    }

    /// Files `held` under the group a `g.*` call is running for. Nothing outside one.
    pub fn note(&self, held: Held) {
        if let Some(id) = self.current.get() {
            self.held.borrow_mut().entry(id).or_default().push(held);
        }
    }

    /// Everything every group holds, which the caller takes back: the config runs again with
    /// no name held, as at start, so an earlier layer can claim what a later one's group had.
    pub fn begin_reload(&self) -> Vec<Held> {
        *self.named.borrow_mut() = Some(BTreeSet::new());
        std::mem::take(&mut *self.held.borrow_mut())
            .into_values()
            .flatten()
            .collect()
    }

    /// The groups nobody named during the reload are forgotten: the plugin that named them is
    /// not loaded any more. What one holds can only have come from a handle the old code kept.
    pub fn end_reload(&self) -> Vec<Held> {
        let named = self.named.borrow_mut().take().unwrap_or_default();
        let mut by_name = self.by_name.borrow_mut();
        let gone: Vec<GroupId> = by_name
            .values()
            .copied()
            .filter(|id| !named.contains(id))
            .collect();
        by_name.retain(|_, id| named.contains(id));
        let mut held = self.held.borrow_mut();
        gone.into_iter()
            .flat_map(|id| held.remove(&id).unwrap_or_default())
            .collect()
    }
}

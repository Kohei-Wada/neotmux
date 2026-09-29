use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use crate::draw::Hl;

/// Every highlight group, by name. The generation moves on every `set`, so a quiet frame can
/// tell that what it drew is stale.
#[derive(Default)]
pub struct HlGroups {
    groups: RefCell<BTreeMap<String, Hl>>,
    generation: Cell<u64>,
}

impl HlGroups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, name: &str, hl: Hl) {
        let mut groups = self.groups.borrow_mut();
        if groups.get(name) == Some(&hl) {
            return;
        }
        groups.insert(name.to_string(), hl);
        self.generation.set(self.generation.get() + 1);
    }

    pub fn get(&self, name: &str) -> Option<Hl> {
        self.groups.borrow().get(name).copied()
    }

    pub fn generation(&self) -> u64 {
        self.generation.get()
    }
}

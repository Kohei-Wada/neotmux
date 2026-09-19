use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::RegistryKey;

use crate::source::SourceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Tick,
    Event(String),
    Key,
    Keymap(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindTag {
    Tick,
    Event,
    Key,
    Keymap,
}

impl Kind {
    pub fn tag(&self) -> KindTag {
        match self {
            Kind::Tick => KindTag::Tick,
            Kind::Event(_) => KindTag::Event,
            Kind::Key => KindTag::Key,
            Kind::Keymap(_) => KindTag::Keymap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationId(u64);

struct Entry {
    owner: SourceId,
    kind: Kind,
    callback: Rc<RegistryKey>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    entries: BTreeMap<RegistrationId, Entry>,
}

#[derive(Default)]
pub struct Registry {
    inner: RefCell<Inner>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, owner: SourceId, kind: Kind, callback: Rc<RegistryKey>) -> RegistrationId {
        let mut inner = self.inner.borrow_mut();
        inner.next += 1;
        let id = RegistrationId(inner.next);
        inner.entries.insert(
            id,
            Entry {
                owner,
                kind,
                callback,
            },
        );
        id
    }

    pub fn remove(&self, id: RegistrationId) -> bool {
        self.inner.borrow_mut().entries.remove(&id).is_some()
    }

    pub fn remove_all_of(&self, owner: SourceId) -> usize {
        let mut inner = self.inner.borrow_mut();
        let doomed: Vec<RegistrationId> = inner
            .entries
            .iter()
            .filter(|(_, e)| e.owner == owner)
            .map(|(id, _)| *id)
            .collect();
        for id in &doomed {
            inner.entries.remove(id);
        }
        doomed.len()
    }

    pub fn snapshot(&self, tag: KindTag) -> Vec<RegistrationId> {
        self.inner
            .borrow()
            .entries
            .iter()
            .filter(|(_, e)| e.kind.tag() == tag)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn owner(&self, id: RegistrationId) -> Option<SourceId> {
        self.inner.borrow().entries.get(&id).map(|e| e.owner)
    }

    pub fn kind(&self, id: RegistrationId) -> Option<Kind> {
        self.inner.borrow().entries.get(&id).map(|e| e.kind.clone())
    }

    pub fn callback(&self, id: RegistrationId) -> Option<Rc<RegistryKey>> {
        self.inner
            .borrow()
            .entries
            .get(&id)
            .map(|e| e.callback.clone())
    }

    pub fn len(&self) -> usize {
        self.inner.borrow().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SourceId, Sources};

    fn key(lua: &mlua::Lua, body: &str) -> std::rc::Rc<mlua::RegistryKey> {
        let f: mlua::Function = lua.load(body).eval().unwrap();
        std::rc::Rc::new(lua.create_registry_value(f).unwrap())
    }

    fn setup() -> (mlua::Lua, Registry, SourceId, SourceId) {
        let lua = crate::vm::new_vm().unwrap();
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let b = sources.intern("beta");
        (lua, Registry::new(), a, b)
    }

    #[test]
    fn a_registration_remembers_its_owner_and_kind() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        assert_eq!(reg.owner(id), Some(a));
        assert_eq!(reg.snapshot(KindTag::Tick), vec![id]);
        assert!(reg.snapshot(KindTag::Key).is_empty());
    }

    #[test]
    fn remove_is_idempotent() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        assert!(reg.remove(id));
        assert!(!reg.remove(id));
        assert!(reg.snapshot(KindTag::Tick).is_empty());
    }

    #[test]
    fn removing_one_source_leaves_the_others_alone() {
        let (lua, reg, a, b) = setup();
        let a_tick = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        let a_key = reg.add(a, Kind::Key, key(&lua, "return function() end"));
        let b_tick = reg.add(b, Kind::Tick, key(&lua, "return function() end"));
        assert_eq!(reg.remove_all_of(a), 2);
        assert_eq!(reg.snapshot(KindTag::Tick), vec![b_tick]);
        assert!(reg.owner(a_tick).is_none());
        assert!(reg.owner(a_key).is_none());
    }

    #[test]
    fn snapshot_is_stable_while_a_callback_mutates_the_registry() {
        let (lua, reg, a, _) = setup();
        let first = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        let ids = reg.snapshot(KindTag::Tick);
        let added_during = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        reg.remove(first);
        assert_eq!(ids, vec![first]);
        assert_eq!(reg.snapshot(KindTag::Tick), vec![added_during]);
    }

    #[test]
    fn remove_all_of_an_owner_with_no_registrations_is_a_no_op() {
        let (_lua, reg, a, _) = setup();
        assert_eq!(reg.remove_all_of(a), 0);
    }

    #[test]
    fn remove_all_of_called_twice_removes_nothing_the_second_time() {
        let (lua, reg, a, _) = setup();
        reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        assert_eq!(reg.remove_all_of(a), 1);
        assert_eq!(reg.remove_all_of(a), 0);
    }

    #[test]
    fn a_kind_carries_its_payload() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(
            a,
            Kind::Keymap("C-b".to_string()),
            key(&lua, "return function() end"),
        );
        assert_eq!(reg.kind(id), Some(Kind::Keymap("C-b".to_string())));
    }

    #[test]
    fn the_callback_outlives_the_registry_borrow() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(a, Kind::Tick, key(&lua, "return function() return 42 end"));
        let cb = reg.callback(id).unwrap();
        let f: mlua::Function = lua.registry_value(&cb).unwrap();
        assert_eq!(f.call::<i64>(()).unwrap(), 42);
    }
}

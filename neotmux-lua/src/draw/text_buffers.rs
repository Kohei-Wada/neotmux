use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use crate::draw::{BufId, BufWindow, TextBuffer};
use crate::plugin::SourceId;

/// Every buffer, who owns it, which window shows which, and the namespaces. The generation
/// moves on any change a frame would show.
#[derive(Default)]
pub struct TextBuffers {
    bufs: RefCell<BTreeMap<BufId, (SourceId, TextBuffer)>>,
    windows: RefCell<BTreeMap<BufWindow, BufId>>,
    namespaces: RefCell<BTreeMap<String, u32>>,
    next: Cell<u32>,
    generation: Cell<u64>,
}

impl TextBuffers {
    pub fn new() -> Self {
        Self::default()
    }

    fn bump(&self) {
        self.generation.set(self.generation.get() + 1);
    }

    pub fn create(&self, owner: SourceId) -> BufId {
        let id = BufId(self.next.get() + 1);
        self.next.set(id.0);
        self.bufs
            .borrow_mut()
            .insert(id, (owner, TextBuffer::default()));
        id
    }

    pub fn exists(&self, id: BufId) -> bool {
        self.bufs.borrow().contains_key(&id)
    }

    pub fn delete(&self, id: BufId) {
        if self.bufs.borrow_mut().remove(&id).is_some() {
            self.windows.borrow_mut().retain(|_, shown| *shown != id);
            self.bump();
        }
    }

    pub fn with<R>(&self, id: BufId, f: impl FnOnce(&mut TextBuffer) -> R) -> Option<R> {
        let out = self.bufs.borrow_mut().get_mut(&id).map(|(_, buf)| f(buf));
        if out.is_some() {
            self.bump();
        }
        out
    }

    pub fn read<R>(&self, id: BufId, f: impl FnOnce(&TextBuffer) -> R) -> Option<R> {
        self.bufs.borrow().get(&id).map(|(_, buf)| f(buf))
    }

    pub fn namespace(&self, name: &str) -> u32 {
        let mut namespaces = self.namespaces.borrow_mut();
        let next = namespaces.len() as u32 + 1;
        *namespaces.entry(name.to_string()).or_insert(next)
    }

    pub(crate) fn show(&self, window: BufWindow, buf: Option<BufId>) {
        let mut windows = self.windows.borrow_mut();
        match buf {
            Some(buf) => windows.insert(window, buf),
            None => windows.remove(&window),
        };
        self.bump();
    }

    pub(crate) fn shown(&self) -> Vec<(BufWindow, BufId)> {
        self.windows
            .borrow()
            .iter()
            .map(|(w, b)| (*w, *b))
            .collect()
    }

    pub fn remove_all_of(&self, owner: SourceId) {
        let gone: Vec<BufId> = self
            .bufs
            .borrow()
            .iter()
            .filter(|(_, (o, _))| *o == owner)
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            self.delete(id);
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.get()
    }
}

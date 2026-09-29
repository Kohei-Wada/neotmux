use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use neotmux_proto::SessionId;

use crate::draw::{BufId, BufWindow, TextBuffer};
use crate::plugin::SourceId;

type Bindings = BTreeMap<(Option<SessionId>, BufWindow), Option<BufId>>;

/// Every buffer, who owns it, which window shows which in which session, and the namespaces.
/// The generation moves on any change a frame would show.
///
/// A window's buffer is bound per session, as Neovim binds a window per tabpage: what was set
/// with no session bound is every session's, and a session's own binding, `None` included,
/// overrides it there.
#[derive(Default)]
pub struct TextBuffers {
    bufs: RefCell<BTreeMap<BufId, (SourceId, TextBuffer)>>,
    windows: RefCell<Bindings>,
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
            self.windows
                .borrow_mut()
                .retain(|_, shown| *shown != Some(id));
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

    pub(crate) fn show(&self, session: Option<SessionId>, window: BufWindow, buf: Option<BufId>) {
        let mut windows = self.windows.borrow_mut();
        match (session, buf) {
            (None, None) => windows.remove(&(None, window)),
            _ => windows.insert((session, window), buf),
        };
        self.bump();
    }

    /// The window is gone, from every session.
    pub(crate) fn close(&self, window: BufWindow) {
        self.windows.borrow_mut().retain(|(_, w), _| *w != window);
        self.bump();
    }

    /// A session is over: its own bindings go with it.
    pub(crate) fn forget(&self, session: SessionId) {
        let mut windows = self.windows.borrow_mut();
        let before = windows.len();
        windows.retain(|(s, _), _| *s != Some(session));
        if windows.len() != before {
            self.bump();
        }
    }

    /// What each window shows in `session`: its own binding where it has one, else the default.
    pub(crate) fn shown_in(&self, session: Option<SessionId>) -> Vec<(BufWindow, BufId)> {
        let windows = self.windows.borrow();
        let mut shown: BTreeMap<BufWindow, Option<BufId>> = BTreeMap::new();
        // The unbound keys sort first, so a session's own binding is inserted over them.
        for ((s, w), b) in windows.iter() {
            if s.is_none() || *s == session {
                shown.insert(*w, *b);
            }
        }
        shown
            .into_iter()
            .filter_map(|(w, b)| b.map(|b| (w, b)))
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

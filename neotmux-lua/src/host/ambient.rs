//! Which session the work in hand is for, and the facts that are that session's.
//!
//! There is one Lua layer for the server, so the geometry a plugin reads has to be the
//! geometry of the session whose tick is running. The API's closures were written against one
//! `Rc<RefCell<Geometry>>` and there is no reason to rewrite them: that cell is the *current*
//! session's, this holds everybody else's, and binding a session swaps the two. The swap moves
//! a `Vec`'s and a `BTreeMap`'s headers, so it costs the same whatever a session holds.

use crate::host::Bound;
use crate::host::{PaneInfos, PaneTtys};
use neotmux_proto::{Geometry, SessionId};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// What one session is holding while some other session is the one bound.
#[derive(Default)]
struct Parked {
    geometry: Geometry,
    pane_info: PaneInfos,
    pane_tty: PaneTtys,
}

pub struct Ambient {
    /// The cells the API's closures hold. Whatever is in them belongs to `current`.
    geometry: Rc<RefCell<Geometry>>,
    pane_info: Rc<RefCell<PaneInfos>>,
    pane_tty: Rc<RefCell<PaneTtys>>,
    current: Cell<Option<SessionId>>,
    parked: RefCell<BTreeMap<SessionId, Parked>>,
    /// What each session is called. Set once, by the host, when it makes one.
    names: RefCell<BTreeMap<SessionId, String>>,
}

impl Ambient {
    pub fn new(
        geometry: Rc<RefCell<Geometry>>,
        pane_info: Rc<RefCell<PaneInfos>>,
        pane_tty: Rc<RefCell<PaneTtys>>,
    ) -> Self {
        Self {
            geometry,
            pane_info,
            pane_tty,
            current: Cell::new(None),
            parked: RefCell::new(BTreeMap::new()),
            names: RefCell::new(BTreeMap::new()),
        }
    }

    /// What a session is called.
    pub fn name(&self, session: SessionId, name: &str) {
        self.names.borrow_mut().insert(session, name.to_string());
    }

    /// The bound session's name, or `None` outside any dispatch — during a config load, where
    /// there is no session to be the name of.
    pub fn current_name(&self) -> Option<String> {
        let current = self.current.get()?;
        self.names.borrow().get(&current).cloned()
    }

    /// Every session there is, in name order, which is the order everything that lists them
    /// prints them in.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.names.borrow().values().cloned().collect();
        names.sort();
        names
    }

    pub fn id_of(&self, name: &str) -> Option<SessionId> {
        self.names
            .borrow()
            .iter()
            .find(|(_, held)| held.as_str() == name)
            .map(|(id, _)| *id)
    }

    /// The session the work in hand is for, or `None` outside any dispatch — during a config
    /// load, for instance, where there is no session yet.
    pub fn current(&self) -> Option<SessionId> {
        self.current.get()
    }

    /// Everything a session was holding, dropped: it is over.
    pub fn forget(&self, session: SessionId) {
        self.parked.borrow_mut().remove(&session);
        self.names.borrow_mut().remove(&session);
    }

    /// Makes `session` the one the live cells belong to, until the guard is dropped.
    pub fn bind(&self, session: SessionId) -> Bound<'_> {
        let was = self.current.get();
        self.swap(Some(session));
        Bound { ambient: self, was }
    }

    /// Puts what the live cells hold back where it came from and brings out `next`'s.
    ///
    /// Nothing is parked under `None`: an unbound layer holds an empty geometry, which is what
    /// a config load has to read, and there is no session for it to be given back to.
    pub(crate) fn swap(&self, next: Option<SessionId>) {
        let was = self.current.replace(next);
        let mut parked = self.parked.borrow_mut();
        let mut held = Parked::default();
        std::mem::swap(&mut held.geometry, &mut self.geometry.borrow_mut());
        std::mem::swap(&mut held.pane_info, &mut self.pane_info.borrow_mut());
        std::mem::swap(&mut held.pane_tty, &mut self.pane_tty.borrow_mut());
        if let Some(was) = was {
            parked.insert(was, held);
        }
        if let Some(next) = next
            && let Some(mut theirs) = parked.remove(&next)
        {
            std::mem::swap(&mut theirs.geometry, &mut self.geometry.borrow_mut());
            std::mem::swap(&mut theirs.pane_info, &mut self.pane_info.borrow_mut());
            std::mem::swap(&mut theirs.pane_tty, &mut self.pane_tty.borrow_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neotmux_proto::Rect;

    fn ambient() -> (Ambient, Rc<RefCell<Geometry>>) {
        let geometry = Rc::new(RefCell::new(Geometry::default()));
        let pane_info = Rc::new(RefCell::new(PaneInfos::new()));
        let pane_tty = Rc::new(RefCell::new(PaneTtys::new()));
        (
            Ambient::new(geometry.clone(), pane_info, pane_tty),
            geometry,
        )
    }

    fn sized(rows: u16) -> Geometry {
        Geometry {
            area: Rect::new(0, 0, 80, rows),
            ..Geometry::default()
        }
    }

    #[test]
    fn nothing_is_bound_until_something_binds_it() {
        let (ambient, _) = ambient();
        assert_eq!(ambient.current(), None);
    }

    /// What a session was left holding is what it is given back, and the live cell is
    /// whichever session is bound right now.
    #[test]
    fn each_session_keeps_its_own_geometry_across_a_bind() {
        let (ambient, live) = ambient();
        let one = SessionId::from(1);
        let two = SessionId::from(2);

        {
            let _bound = ambient.bind(one);
            *live.borrow_mut() = sized(10);
        }
        {
            let _bound = ambient.bind(two);
            assert_eq!(
                live.borrow().area.h,
                0,
                "session two was shown session one's geometry"
            );
            *live.borrow_mut() = sized(20);
        }
        let _bound = ambient.bind(one);
        assert_eq!(live.borrow().area.h, 10);
    }

    #[test]
    fn a_bind_inside_a_bind_puts_the_outer_one_back() {
        let (ambient, live) = ambient();
        let outer = SessionId::from(1);
        let inner = SessionId::from(2);

        let _bound = ambient.bind(outer);
        *live.borrow_mut() = sized(10);
        {
            let _inner = ambient.bind(inner);
            assert_eq!(ambient.current(), Some(inner));
            *live.borrow_mut() = sized(20);
        }
        assert_eq!(ambient.current(), Some(outer));
        assert_eq!(live.borrow().area.h, 10);
    }

    /// The guard is what restores, so an early return or an error on the way out of a dispatch
    /// cannot leave another session's facts in the live cell.
    #[test]
    fn dropping_the_guard_restores_even_when_the_work_did_not_finish() {
        let (ambient, live) = ambient();
        let one = SessionId::from(1);
        {
            let _bound = ambient.bind(one);
            *live.borrow_mut() = sized(10);
        }
        assert_eq!(ambient.current(), None);
        assert_eq!(
            live.borrow().area.h,
            0,
            "the unbound cell still held a session's geometry"
        );
    }
}

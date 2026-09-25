//! The timers that are armed, and which of them this frame owes a call.
//!
//! A frame is the only thing that happens here, so a timer is a deadline compared against the
//! instant the frame happened at. Nothing sleeps, nothing is scheduled, and a timer that came
//! due while the VM was busy is due once rather than once per interval it missed — five
//! callbacks in a row to make up for a slow frame is a worse answer than one.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::RegistryKey;
use neotmux_proto::SessionId;

use crate::plugin::SourceId;
use crate::sched::{CountsOverruns, SessionSlot, Strikes, SweepFlag, Sweeping};

/// The shortest interval that means anything: the frame. Asking for less is asking for every
/// frame, which is what this clamps it to.
pub const FRAME: Duration = Duration::from_millis(16);

/// What `timer.every` and `timer.after` answered with, and what `timer:stop()` names. Minted
/// here rather than by the host for the reason [`crate::sched::JobId`] gives: the call has to
/// answer before the frame ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerId(pub u64);

struct Timer {
    owner: SourceId,
    session: SessionSlot,
    interval: Duration,
    /// False for a one-shot, which fires once and is then spent.
    repeating: bool,
    due_at: Instant,
    callback: Rc<RegistryKey>,
    /// A one-shot whose callback has been *called*, not merely listed as due. It is kept
    /// until the next frame rather than dropped where it fired, so that a `stop` during the
    /// rest of that pass is answered by the same entry. Marked by `called`, never by `due`:
    /// between the two a stop can still cancel the call, and it has to be able to say so.
    spent: bool,
    overruns: Strikes,
}

/// One timer this frame owes a call, and who to report a raising callback against.
pub struct Due {
    pub id: TimerId,
    pub owner: SourceId,
    pub callback: Rc<RegistryKey>,
}

#[derive(Default)]
pub struct Timers {
    next: Cell<u64>,
    armed: RefCell<BTreeMap<TimerId, Timer>>,
    sweeping: SweepFlag,
}

impl Timers {
    /// Says that a session is being swept, for as long as the guard lives; `arm` refuses
    /// while it does, for the reason [`SweepFlag`] gives.
    pub fn sweeping(&self) -> Sweeping<'_> {
        self.sweeping.raise()
    }

    /// Arms one, due `ms` from `now`, or says why not — one line, no punctuation at the end,
    /// the shape `job.start` already answers with. An interval under a frame is raised to
    /// one: the host has nothing finer than a frame to fire on, so promising less would be a
    /// promise it could not keep.
    pub fn arm(
        &self,
        ms: u64,
        repeating: bool,
        owner: SourceId,
        session: Option<SessionId>,
        callback: Rc<RegistryKey>,
        now: Instant,
    ) -> Result<TimerId, String> {
        if self.sweeping.is_raised() {
            return Err("the session is ending".to_string());
        }
        let interval = Duration::from_millis(ms).max(FRAME);
        let id = TimerId(self.next.get() + 1);
        self.next.set(id.0);
        self.armed.borrow_mut().insert(
            id,
            Timer {
                owner,
                session: SessionSlot::new(session),
                interval,
                repeating,
                due_at: now + interval,
                callback,
                spent: false,
                overruns: Strikes::default(),
            },
        );
        Ok(id)
    }

    /// Disarms one, answering whether the call it was armed for will now not happen. A
    /// one-shot whose callback has already run, and a second stop, both answer false: there
    /// is nothing left that this cancelled. A one-shot cancelled after `due` listed it but
    /// before it was called answers true, because the call really does not happen.
    pub fn stop(&self, id: TimerId) -> bool {
        match self.armed.borrow_mut().remove(&id) {
            Some(timer) => !timer.spent,
            None => false,
        }
    }

    /// Whether the timer has not been stopped. `due` takes its whole list before any callback
    /// runs, so the caller asks this again between calls: a timer that stopped itself — or
    /// another one — from inside a callback must not be called in the pass it stopped in.
    pub fn is_stopped(&self, id: TimerId) -> bool {
        !self.armed.borrow().contains_key(&id)
    }

    /// Records that a due timer's callback has now been called. A one-shot is spent by this
    /// and by nothing else, which is what lets `stop` tell "cancelled in time" from "too
    /// late". A repeating one has nothing to record: `due` already moved its deadline.
    pub fn called(&self, id: TimerId) {
        if let Some(timer) = self.armed.borrow_mut().get_mut(&id)
            && !timer.repeating
        {
            timer.spent = true;
        }
    }

    /// Disarms every timer of a source that is going away: the callback belongs to a chunk
    /// that is not there any more.
    pub fn remove_all_of(&self, owner: SourceId) {
        self.armed
            .borrow_mut()
            .retain(|_, timer| timer.owner != owner);
    }

    /// The same for a session that is over, spent one-shots included — a one-shot lingers a
    /// frame past its call, and a session that is gone has no frame left to linger into. A
    /// timer still waiting to be adopted belongs to no session yet and is left alone.
    pub fn remove_all_for(&self, session: SessionId) {
        self.armed
            .borrow_mut()
            .retain(|_, timer| !timer.session.is(session));
    }

    /// The timers of `session` that are due at `now`, and the bookkeeping that goes with
    /// having listed them: a repeating one's next deadline is `now + interval` — not
    /// `due_at + interval`, which is what would produce the catch-up burst. A one-shot is
    /// left alone until `called`, since until then it may still be stopped.
    pub fn due(&self, session: SessionId, now: Instant) -> Vec<Due> {
        let mut out = Vec::new();
        let mut armed = self.armed.borrow_mut();
        // Last frame's one-shots, whose pass is over and who therefore have nothing left to
        // answer for. A one-shot that was listed but never called — stopped mid-pass — is
        // already gone, removed by the stop itself.
        armed.retain(|_, timer| !timer.spent);
        for (id, timer) in armed.iter_mut() {
            if !timer.session.claim(session) {
                continue;
            }
            if now < timer.due_at {
                continue;
            }
            out.push(Due {
                id: *id,
                owner: timer.owner,
                callback: Rc::clone(&timer.callback),
            });
            if timer.repeating {
                timer.due_at = now + timer.interval;
            }
        }
        out
    }
}

impl CountsOverruns for Timers {
    type Id = TimerId;
    const WHAT: &'static str = "timer";
    const ENDED: &'static str = "stopped";

    fn forgive(&self, id: TimerId) {
        if let Some(timer) = self.armed.borrow_mut().get_mut(&id) {
            timer.overruns.forgive();
        }
    }

    /// Disarms the timer once it has run over `limit` times in a row. A stopped timer does not
    /// come back: the plugin arms a new one or it does not.
    fn overran(&self, id: TimerId, limit: u32) -> bool {
        let mut armed = self.armed.borrow_mut();
        let Some(timer) = armed.get_mut(&id) else {
            return false;
        };
        if !timer.overruns.strike(limit) {
            return false;
        }
        armed.remove(&id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::ANONYMOUS;

    fn callback(lua: &mlua::Lua) -> Rc<RegistryKey> {
        let function = lua.create_function(|_, ()| Ok(())).unwrap();
        Rc::new(lua.create_registry_value(function).unwrap())
    }

    fn session() -> SessionId {
        SessionId::from(1)
    }

    /// The deadline is reached, not passed: a frame landing exactly on it is the frame the
    /// timer comes due in. A frame is only ever *near* the deadline in the real thing, so the
    /// boundary is pinned here, where the clock is the test's.
    #[test]
    fn a_frame_landing_exactly_on_the_deadline_fires() {
        let lua = mlua::Lua::new();
        let timers = Timers::default();
        let start = Instant::now();
        let id = timers
            .arm(16, false, ANONYMOUS, Some(session()), callback(&lua), start)
            .unwrap();

        assert!(
            timers
                .due(session(), start + Duration::from_millis(15))
                .is_empty(),
            "fired a millisecond early"
        );
        let due = timers.due(session(), start + Duration::from_millis(16));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id);
    }

    /// An arm during a sweep is refused, and the refusal lasts exactly as long as the guard:
    /// a timer armed into a session that is going away would be fired by no frame and swept
    /// by no sweep, and it would hold its callback forever.
    #[test]
    fn arming_while_a_session_is_being_swept_is_refused() {
        let lua = mlua::Lua::new();
        let timers = Timers::default();
        let start = Instant::now();

        {
            let _sweeping = timers.sweeping();
            let refused = timers.arm(16, false, ANONYMOUS, Some(session()), callback(&lua), start);
            assert_eq!(refused, Err("the session is ending".to_string()));
        }

        assert!(
            timers
                .arm(16, false, ANONYMOUS, Some(session()), callback(&lua), start)
                .is_ok(),
            "the guard went on refusing after the sweep was over"
        );
    }
}

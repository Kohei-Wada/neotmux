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

use crate::source::SourceId;

/// The shortest interval that means anything: the frame. Asking for less is asking for every
/// frame, which is what this clamps it to.
pub const FRAME: Duration = Duration::from_millis(16);

/// What `timer.every` and `timer.after` answered with, and what `timer:stop()` names. Minted
/// here rather than by the host for the reason [`crate::jobs::JobId`] gives: the call has to
/// answer before the frame ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerId(pub u64);

struct Timer {
    owner: SourceId,
    /// None only between a load-time arm and the first frame, which adopts it.
    session: Option<SessionId>,
    interval: Duration,
    /// False for a one-shot, which fires once and is then spent.
    repeating: bool,
    due_at: Instant,
    callback: Rc<RegistryKey>,
    /// A one-shot that has fired. It is kept until the next frame rather than dropped where
    /// it fired, so that everything asked about it during that pass — `is_armed` before its
    /// own call, a `stop` from another callback — answers about the same entry.
    spent: bool,
}

/// One timer this frame owes a call, and who to report a raising callback against.
pub struct Due {
    pub timer: TimerId,
    pub owner: SourceId,
    pub callback: Rc<RegistryKey>,
}

#[derive(Default)]
pub struct Timers {
    next: Cell<u64>,
    armed: RefCell<BTreeMap<TimerId, Timer>>,
}

impl Timers {
    /// Arms one, due `ms` from `now`. An interval under a frame is raised to one: the host
    /// has nothing finer than a frame to fire on, so promising less would be a promise it
    /// could not keep.
    pub fn arm(
        &self,
        ms: u64,
        repeating: bool,
        owner: SourceId,
        session: Option<SessionId>,
        callback: Rc<RegistryKey>,
        now: Instant,
    ) -> TimerId {
        let interval = Duration::from_millis(ms).max(FRAME);
        let id = TimerId(self.next.get() + 1);
        self.next.set(id.0);
        self.armed.borrow_mut().insert(
            id,
            Timer {
                owner,
                session,
                interval,
                repeating,
                due_at: now + interval,
                callback,
                spent: false,
            },
        );
        id
    }

    /// Disarms one, answering whether there was anything to disarm. A one-shot that has
    /// already fired, and a second stop, both answer false: there is no timer left to name.
    pub fn stop(&self, id: TimerId) -> bool {
        match self.armed.borrow_mut().remove(&id) {
            Some(timer) => !timer.spent,
            None => false,
        }
    }

    /// Whether the timer has not been stopped. `due` takes its whole list before any callback
    /// runs, so the caller asks this again between calls: a timer that stopped itself — or
    /// another one — from inside a callback must not be called in the pass it stopped in. A
    /// one-shot that is spent but unstopped is still owed the call it was listed for.
    pub fn is_armed(&self, id: TimerId) -> bool {
        self.armed.borrow().contains_key(&id)
    }

    /// The timers of `session` that are due at `now`, and the bookkeeping that goes with
    /// having fired them: a repeating one's next deadline is `now + interval` — not
    /// `due_at + interval`, which is what would produce the catch-up burst — and a one-shot
    /// is spent.
    pub fn due(&self, session: SessionId, now: Instant) -> Vec<Due> {
        let mut out = Vec::new();
        let mut armed = self.armed.borrow_mut();
        // Last frame's one-shots, whose pass is over and who therefore have nothing left to
        // answer for.
        armed.retain(|_, timer| !timer.spent);
        for (id, timer) in armed.iter_mut() {
            // A timer armed before any session existed is adopted by the first frame to
            // come looking, and belongs to that session from then on.
            match timer.session {
                Some(owner) if owner != session => continue,
                Some(_) => {}
                None => timer.session = Some(session),
            }
            if now < timer.due_at {
                continue;
            }
            out.push(Due {
                timer: *id,
                owner: timer.owner,
                callback: Rc::clone(&timer.callback),
            });
            if timer.repeating {
                timer.due_at = now + timer.interval;
            } else {
                timer.spent = true;
            }
        }
        out
    }
}

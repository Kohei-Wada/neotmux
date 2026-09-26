use std::rc::Rc;

use neotmux_proto::screen::PaneRead;

use crate::api::forget_if_no_longer_drawing;
use crate::draw::{Buffers, Floats, Reservations};
use crate::plugin::{Commands, Held, Registry, SourceId};
use crate::sched::Jobs;
use crate::sched::timers::Timers;

/// Everything a source that failed to load, or a group being emptied, has to be taken back
/// out of, as one value: the callers otherwise take eight collaborators.
#[derive(Clone)]
pub(crate) struct Sweep {
    pub(crate) registry: Rc<Registry>,
    pub(crate) buffers: Rc<Buffers>,
    pub(crate) reservations: Rc<Reservations>,
    pub(crate) floats: Rc<Floats>,
    pub(crate) jobs: Rc<Jobs>,
    pub(crate) timers: Rc<Timers>,
    pub(crate) commands: Rc<Commands>,
    pub(crate) panes: Rc<dyn PaneRead>,
}

impl Sweep {
    /// What a chunk that is not there registered, reserved, drew, started and armed. Jobs and
    /// timers are killed and stopped rather than left to finish: the callbacks that would hear
    /// how they ended belong to the chunk that is going away.
    pub(crate) fn remove_all_of(&self, id: SourceId) {
        self.registry.remove_all_of(id);
        self.reservations.remove_all_of(id);
        self.floats.remove_all_of(id);
        self.jobs.remove_all_of(id);
        self.timers.remove_all_of(id);
        forget_if_no_longer_drawing(&self.registry, &self.buffers, id);
    }

    /// Takes back one group's worth. A source left with no tick loses its committed frame,
    /// as `handle:remove()` has it lose it.
    pub(crate) fn empty(&self, held: Vec<Held>) {
        let mut owners = Vec::new();
        for one in held {
            match one {
                Held::Registration(id) => {
                    if let Some(owner) = self.registry.owner(id) {
                        owners.push(owner);
                    }
                    self.registry.remove(id);
                }
                Held::Reservation(id) => {
                    self.reservations.remove(id);
                }
                Held::Float(id) => self.floats.remove(id),
                Held::Job(id) => self.jobs.remove(id),
                Held::Timer(id) => {
                    self.timers.stop(id);
                }
                Held::Freeze(hold) => {
                    self.panes.release(hold);
                }
                Held::Command { name, callback } => {
                    self.commands.remove_if(&name, &callback);
                }
            }
        }
        owners.sort();
        owners.dedup();
        for owner in owners {
            forget_if_no_longer_drawing(&self.registry, &self.buffers, owner);
        }
    }
}

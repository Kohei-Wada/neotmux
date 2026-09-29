use std::rc::Rc;

use mlua::RegistryKey;
use neotmux_proto::screen::Hold;

use crate::draw::{BufId, FloatId, ReservationId};
use crate::plugin::RegistrationId;
use crate::sched::JobId;
use crate::sched::timers::TimerId;

/// One thing a group holds on the host, and what taking it back means.
pub enum Held {
    Registration(RegistrationId),
    Reservation(ReservationId),
    PaneBand(ReservationId),
    Float(FloatId),
    Buf(BufId),
    Job(JobId),
    Timer(TimerId),
    Freeze(Hold),
    /// The callback as well as the name, so a command the source registered again since is
    /// not the one taken away.
    Command {
        name: String,
        callback: Rc<RegistryKey>,
    },
}

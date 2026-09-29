use std::rc::Rc;

use mlua::RegistryKey;

use crate::plugin::SourceId;
use crate::sched::JobId;

/// One thing the frame has to do for a job: call a callback, or end it.
pub enum Delivery {
    Lines {
        job: JobId,
        owner: SourceId,
        callback: Rc<RegistryKey>,
        lines: Vec<String>,
    },
    Exit {
        job: JobId,
        owner: SourceId,
        callback: Option<Rc<RegistryKey>>,
        code: Option<i32>,
        stdout: Option<String>,
        stderr: Option<String>,
        err: Option<String>,
    },
}

impl Delivery {
    /// Which job this is for, so an overrunning callback is counted against it and no other.
    pub fn job(&self) -> JobId {
        match self {
            Delivery::Lines { job, .. } | Delivery::Exit { job, .. } => *job,
        }
    }

    /// Who a raising or overrunning callback is reported against.
    pub fn owner(&self) -> SourceId {
        match self {
            Delivery::Lines { owner, .. } | Delivery::Exit { owner, .. } => *owner,
        }
    }
}

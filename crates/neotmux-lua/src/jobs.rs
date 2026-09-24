//! The jobs that are running, and what is owed to whoever started each one.
//!
//! The processes themselves are the host's (see [`crate::runner`]); what is here is the part
//! a test can pin down: who owns a job, which session's frame its callbacks belong in, what
//! has been collected, and what is said when it ends.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::RegistryKey;
use neotmux_proto::SessionId;

use crate::runner::{JobChunk, JobSpec, Runner, Running};
use crate::source::SourceId;

/// What `job.start` answered with, and what `job:kill()` names. Minted here rather than by
/// the host for the reason `WindowIds` gives: the call has to answer before the frame ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct JobId(pub u64);

/// The callbacks a job was started with. Each is a `RegistryKey` because a Lua function
/// cannot outlive a borrow of the `Lua` it came from.
pub struct Callbacks {
    pub on_stdout: Option<Rc<RegistryKey>>,
    pub on_stderr: Option<Rc<RegistryKey>>,
    pub on_exit: Option<Rc<RegistryKey>>,
}

struct Job {
    owner: SourceId,
    /// None only between a load-time `start` and the first frame, which adopts it.
    session: Option<SessionId>,
    buffered: bool,
    running: Box<dyn Running>,
    callbacks: Callbacks,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// One thing the frame has to do for a job: call a callback, or end it.
pub enum Delivery {
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

pub struct Jobs {
    runner: Rc<dyn Runner>,
    next: Cell<u64>,
    live: RefCell<BTreeMap<JobId, Job>>,
}

impl Jobs {
    pub fn new(runner: Rc<dyn Runner>) -> Self {
        Self {
            runner,
            next: Cell::new(1),
            live: RefCell::new(BTreeMap::new()),
        }
    }

    /// Starts one, or says why not. The message is for a plugin to read: one line, no
    /// punctuation at the end, the shape `keymap.set` already answers with.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        spec: JobSpec,
        owner: SourceId,
        session: Option<SessionId>,
        buffered: bool,
        callbacks: Callbacks,
        concurrency: usize,
    ) -> Result<JobId, String> {
        let running_for_owner = self
            .live
            .borrow()
            .values()
            .filter(|job| job.owner == owner)
            .count();
        if running_for_owner >= concurrency {
            return Err(format!("too many jobs: {concurrency} already running"));
        }
        let running = self
            .runner
            .start(&spec)
            .map_err(|err| format!("could not start {}: {err}", spec.cmd.join(" ")))?;
        let id = JobId(self.next.get());
        self.next.set(id.0 + 1);
        self.live.borrow_mut().insert(
            id,
            Job {
                owner,
                session,
                buffered,
                running,
                callbacks,
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        );
        Ok(id)
    }

    /// True when this call is what ended it. The `Exit` delivery it leaves behind is picked
    /// up by the next `deliver`, so a caller hears the end of its own kill.
    pub fn kill(&self, id: JobId) -> bool {
        // Task 3 fills this in; Task 2 needs only the counter and `start`.
        let mut live = self.live.borrow_mut();
        match live.get_mut(&id) {
            Some(job) => {
                job.running.kill();
                true
            }
            None => false,
        }
    }

    /// Everything this session's jobs have to say this frame, oldest job first.
    pub fn deliver(&self, session: SessionId) -> Vec<Delivery> {
        let mut out = Vec::new();
        let mut finished = Vec::new();
        for (id, job) in self.live.borrow_mut().iter_mut() {
            // A job started before any session existed is adopted by the first frame to come
            // looking, and belongs to that session from then on.
            match job.session {
                Some(owner) if owner != session => continue,
                Some(_) => {}
                None => job.session = Some(session),
            }
            for chunk in job.running.poll() {
                match chunk {
                    JobChunk::Stdout(bytes) => job.stdout.extend_from_slice(&bytes),
                    JobChunk::Stderr(bytes) => job.stderr.extend_from_slice(&bytes),
                    JobChunk::Exited(code) => {
                        finished.push((*id, Some(code), None));
                        break;
                    }
                    JobChunk::Failed(why) => {
                        finished.push((*id, None, Some(why)));
                        break;
                    }
                }
            }
        }
        for (id, code, err) in finished {
            let Some(job) = self.live.borrow_mut().remove(&id) else {
                continue;
            };
            let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
            out.push(Delivery::Exit {
                job: id,
                owner: job.owner,
                callback: job.callbacks.on_exit,
                code,
                stdout: job.buffered.then(|| text(job.stdout)),
                stderr: job.buffered.then(|| text(job.stderr)),
                err,
            });
        }
        out
    }
}

//! The jobs that are running, and what is owed to whoever started each one.
//!
//! The processes themselves are the host's (see [`crate::host::Runner`]); what is here is the part
//! a test can pin down: who owns a job, which session's frame its callbacks belong in, what
//! has been collected, and what is said when it ends.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use neotmux_proto::SessionId;

use crate::host::{JobChunk, JobSpec, Runner, Running};
use crate::plugin::SourceId;
use crate::sched::{
    Callbacks, CountsOverruns, Delivery, JobId, SessionSlot, Strikes, SweepFlag, Sweeping,
};

struct Job {
    id: JobId,
    owner: SourceId,
    session: SessionSlot,
    buffered: bool,
    running: Box<dyn Running>,
    callbacks: Callbacks,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// In streaming mode, what has arrived since the last line break: a chunk boundary is not
    /// a line boundary, so a plugin is never handed the half of a line that came first.
    held_stdout: Vec<u8>,
    held_stderr: Vec<u8>,
    /// Set the moment the job is over, whatever ended it. The `Exit` itself is left to the
    /// next `deliver`, so that every path — exited, failed, killed, over its limit — ends the
    /// job in one place and therefore exactly once.
    ending: Option<(Option<i32>, Option<String>)>,
    overruns: Strikes,
}

impl Job {
    /// What the job is making the host keep. In buffered mode that is everything it has said,
    /// since `on_exit` is owed all of it at once; in streaming mode it is only what is still
    /// unhandable — the bytes since the last line break. Complete lines are handed over and
    /// dropped before the frame is out, and counting them would kill the endless stream that
    /// streaming mode exists for.
    fn bytes_held(&self) -> usize {
        if self.buffered {
            self.stdout.len() + self.stderr.len()
        } else {
            fragment(&self.held_stdout) + fragment(&self.held_stderr)
        }
    }

    /// Every complete line that has arrived, leaving the unterminated remainder held.
    fn stream(&mut self, out: &mut Vec<Delivery>) {
        for (held, callback) in [
            (&mut self.held_stdout, &self.callbacks.on_stdout),
            (&mut self.held_stderr, &self.callbacks.on_stderr),
        ] {
            let lines = split_lines(held);
            if lines.is_empty() {
                continue;
            }
            let Some(callback) = callback else {
                continue;
            };
            out.push(Delivery::Lines {
                job: self.id,
                owner: self.owner,
                callback: Rc::clone(callback),
                lines,
            });
        }
    }

    /// What is left when there will be no more line break: one last line rather than nothing,
    /// since a program whose last line is unterminated still said it.
    fn flush(&mut self, out: &mut Vec<Delivery>) {
        for (held, callback) in [
            (&mut self.held_stdout, &self.callbacks.on_stdout),
            (&mut self.held_stderr, &self.callbacks.on_stderr),
        ] {
            if held.is_empty() {
                continue;
            }
            let line = String::from_utf8_lossy(held).into_owned();
            held.clear();
            let Some(callback) = callback else {
                continue;
            };
            out.push(Delivery::Lines {
                job: self.id,
                owner: self.owner,
                callback: Rc::clone(callback),
                lines: vec![line],
            });
        }
    }
}

/// The `err` a job the host ended is given: why it was ended, unless the kill itself failed, in
/// which case the program may be running on and saying `killed` would be a lie.
fn ended_by(killed: std::io::Result<()>, why: &str) -> String {
    match killed {
        Ok(()) => why.to_string(),
        Err(err) => format!("{why}, but the kill failed: {err}"),
    }
}

/// How much of a held buffer is not yet a line: everything after the last break, or all of it
/// if there has been no break at all.
fn fragment(held: &[u8]) -> usize {
    match held.iter().rposition(|byte| *byte == b'\n') {
        Some(at) => held.len() - at - 1,
        None => held.len(),
    }
}

/// Splits off every line that is complete, and leaves the rest. A `\r` immediately before the
/// `\n` belongs to the break rather than to the line: a program that ends its lines the DOS way
/// means the same lines as one that does not.
fn split_lines(held: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    for at in 0..held.len() {
        if held[at] != b'\n' {
            continue;
        }
        let mut end = at;
        if end > start && held[end - 1] == b'\r' {
            end -= 1;
        }
        lines.push(String::from_utf8_lossy(&held[start..end]).into_owned());
        start = at + 1;
    }
    held.drain(..start);
    lines
}

pub struct Jobs {
    runner: Rc<dyn Runner>,
    next: Cell<u64>,
    live: RefCell<BTreeMap<JobId, Job>>,
    sweeping: SweepFlag,
}

impl Jobs {
    pub fn new(runner: Rc<dyn Runner>) -> Self {
        Self {
            runner,
            next: Cell::new(1),
            live: RefCell::new(BTreeMap::new()),
            sweeping: SweepFlag::default(),
        }
    }

    /// Says that a session's jobs are being ended, for as long as the guard lives; `start`
    /// refuses while it does, for the reason [`SweepFlag`] gives.
    pub fn sweeping(&self) -> Sweeping<'_> {
        self.sweeping.raise()
    }

    /// Starts one, or says why not. The message is for a plugin to read: one line, no
    /// punctuation at the end, the shape `keymap.set` already answers with.
    pub fn start(
        &self,
        spec: JobSpec,
        owner: SourceId,
        session: Option<SessionId>,
        buffered: bool,
        callbacks: Callbacks,
        concurrency: usize,
    ) -> Result<JobId, String> {
        if self.sweeping.is_raised() {
            return Err("the session is ending".to_string());
        }
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
                id,
                owner,
                session: SessionSlot::new(session),
                buffered,
                running,
                callbacks,
                stdout: Vec::new(),
                stderr: Vec::new(),
                held_stdout: Vec::new(),
                held_stderr: Vec::new(),
                ending: None,
                overruns: Strikes::default(),
            },
        );
        Ok(id)
    }

    /// Signals the process and marks the job as over, answering whether there was a live job
    /// to end. The `Exit` itself is the next `deliver`'s: a killed job owes its caller the
    /// same one ending every other job owes, and a second kill is not a second ending.
    pub fn kill(&self, id: JobId) -> bool {
        let mut live = self.live.borrow_mut();
        match live.get_mut(&id) {
            Some(job) if job.ending.is_none() => {
                job.ending = Some((None, Some(ended_by(job.running.kill(), "killed"))));
                true
            }
            _ => false,
        }
    }

    /// Kills and drops every job of a source that is going away, with no `Exit` delivery: the
    /// callback that would receive it belongs to the chunk that is not there any more, and
    /// calling into it is worse than not calling.
    pub fn remove_all_of(&self, owner: SourceId) {
        for mut job in self.take_matching(|job| job.owner == owner) {
            let _ = job.running.kill();
        }
    }

    /// Kills and drops one job with no `Exit`, for the reason `remove_all_of` gives: its group
    /// is being emptied, and the callback that would hear the ending is the old code's.
    pub fn remove(&self, id: JobId) {
        for mut job in self.take_matching(|job| job.id == id) {
            let _ = job.running.kill();
        }
    }

    pub fn len(&self) -> usize {
        self.live.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The same for a session that is over, but with the ending each job is still owed: the
    /// chunk that wrote `on_exit` is very much alive here, and a plugin holding a lock for the
    /// length of a job has to learn that the job is over. A job whose `ending` was already set
    /// but never emitted reports that ending rather than this one, so a budget kill a frame
    /// earlier is not swallowed by the session going away a moment later. A job still waiting
    /// to be adopted belongs to no session yet and is left alone.
    pub fn end_session(&self, session: SessionId) -> Vec<Delivery> {
        let mut out = Vec::new();
        for mut job in self.take_matching(|job| job.session.is(session)) {
            let killed = job.running.kill();
            // Before the ending, as `deliver` does it: a session going away is no reason for
            // the plugin to lose the last thing the job said.
            if !job.buffered {
                job.flush(&mut out);
            }
            let (code, err) = job
                .ending
                .take()
                .unwrap_or_else(|| (None, Some(ended_by(killed, "session ended"))));
            let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
            out.push(Delivery::Exit {
                job: job.id,
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

    /// Takes the matching jobs out of the map, so that what is done with them next — killed
    /// and dropped, or killed and reported — cannot be reached twice.
    fn take_matching(&self, doomed: impl Fn(&Job) -> bool) -> Vec<Job> {
        let mut live = self.live.borrow_mut();
        let ids: Vec<JobId> = live
            .iter()
            .filter(|(_, job)| doomed(job))
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .map(|id| live.remove(&id).expect("an id just taken from the map"))
            .collect()
    }

    /// Everything this session's jobs have to say this frame, oldest job first. The limit is
    /// read per frame rather than kept, the way the tick budget is: a plugin that raises it
    /// means it from the next frame on.
    pub fn deliver(&self, session: SessionId, limit: usize) -> Vec<Delivery> {
        let mut out = Vec::new();
        let mut finished = Vec::new();
        for (id, job) in self.live.borrow_mut().iter_mut() {
            if !job.session.claim(session) {
                continue;
            }
            // A job marked as over is never polled again: what it has said is already what it
            // will be reported as having said.
            if job.ending.is_none() {
                for chunk in job.running.poll() {
                    match chunk {
                        JobChunk::Stdout(bytes) => {
                            if job.buffered {
                                job.stdout.extend_from_slice(&bytes);
                            } else {
                                job.held_stdout.extend_from_slice(&bytes);
                            }
                        }
                        JobChunk::Stderr(bytes) => {
                            if job.buffered {
                                job.stderr.extend_from_slice(&bytes);
                            } else {
                                job.held_stderr.extend_from_slice(&bytes);
                            }
                        }
                        JobChunk::Exited(code) => {
                            job.ending = Some((Some(code), None));
                            break;
                        }
                        JobChunk::Failed(why) => {
                            job.ending = Some((None, Some(why)));
                            break;
                        }
                    }
                    if job.bytes_held() > limit {
                        let killed = job.running.kill();
                        job.ending = Some((None, Some(ended_by(killed, "output limit"))));
                        break;
                    }
                }
            }
            if !job.buffered {
                job.stream(&mut out);
            }
            // The ending travels with the id rather than staying on the job, so that there is
            // no way to reach the removal below without one to report.
            if let Some(ending) = job.ending.take() {
                finished.push((*id, ending));
            }
        }
        for (id, (code, err)) in finished {
            let mut job = self
                .live
                .borrow_mut()
                .remove(&id)
                .expect("a job that ended this frame is still in the map that named it");
            // Before the ending, not after it: `on_exit` is the last thing a plugin hears.
            if !job.buffered {
                job.flush(&mut out);
            }
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

impl CountsOverruns for Jobs {
    type Id = JobId;
    const WHAT: &'static str = "job";
    const ENDED: &'static str = "killed";

    fn forgive(&self, id: JobId) {
        if let Some(job) = self.live.borrow_mut().get_mut(&id) {
            job.overruns.forgive();
        }
    }

    /// Ends the job once it has run over `limit` times in a row. The ending is the same
    /// `ending` marker every other ending uses, so `on_exit` is still owed exactly one call —
    /// with no code, and `budget` as the reason.
    fn overran(&self, id: JobId, limit: u32) -> bool {
        let mut live = self.live.borrow_mut();
        let Some(job) = live.get_mut(&id) else {
            return false;
        };
        if job.ending.is_some() || !job.overruns.strike(limit) {
            return false;
        }
        job.ending = Some((None, Some(ended_by(job.running.kill(), "budget"))));
        true
    }
}

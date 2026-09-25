//! The host's half of [`neotmux_lua::runner`]: a program in a real process.
//!
//! The bytes take the path a pane's already take — a reader thread per pipe, a bounded
//! channel, and a nudge on the one wakeup the serving loop is already waiting on. That is
//! what lets a job add no file descriptor to the poll set and nothing to the loop's
//! bookkeeping: the frame that drains the panes drains the jobs too.

use std::io::{self, Read};
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;

use neotmux_lua::host::{JobChunk, JobSpec, Runner, Running};
use neotmux_pty::child::Waker;

const OUTPUT_CHANNEL_BOUND: usize = 64;

/// Somewhere to start a program: the serving loop's waker, and nothing else. One of these
/// serves every session, the way one `Wakeup` serves every pane.
pub struct Processes {
    waker: Waker,
}

impl Processes {
    pub fn new(waker: Waker) -> Self {
        Self { waker }
    }
}

impl Runner for Processes {
    fn start(&self, spec: &JobSpec) -> io::Result<Box<dyn Running>> {
        let (program, args) = spec
            .cmd
            .split_first()
            .ok_or_else(|| io::Error::other("a job needs a command"))?;
        let mut command = Command::new(program);
        command
            .args(args)
            // Added, never `env_clear`: a plugin cannot read the environment, so it has no
            // way to compose a whole one.
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group, so that a kill reaches what the program started as well
            // as the program. `{ 'sh', '-c', 'a | b' }` is the shape a plugin writes, and
            // signalling only the `sh` leaves `a` and `b` holding the write ends of the pipes
            // this job is read through — two reader threads blocked in `read` for the life of
            // the server, and the `sh` never reaped.
            .process_group(0);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn()?;
        let group = Pid::from_raw(child.id() as i32);
        let pipes = child.stdout.take().zip(child.stderr.take());
        let Some((stdout, stderr)) = pipes else {
            abandon(child, group);
            return Err(io::Error::other("the job was given no pipes"));
        };

        let (tx, chunks) = sync_channel(OUTPUT_CHANNEL_BOUND);
        // A thread that could not be spawned takes the child with it: `Child`'s own `Drop`
        // neither kills nor waits, so a job left half-started would run on and become a
        // zombie nobody reaps, while the plugin is told it never started at all.
        let out = match reader("stdout", stdout, JobChunk::Stdout, &tx, &self.waker) {
            Ok(reader) => reader,
            Err(err) => {
                abandon(child, group);
                return Err(err);
            }
        };
        let readers = match reader("stderr", stderr, JobChunk::Stderr, &tx, &self.waker) {
            // The reader already running ends on its own once the child it is reading is
            // gone, which is what `abandon` sees to.
            Ok(reader) => [out, reader],
            Err(err) => {
                abandon(child, group);
                return Err(err);
            }
        };

        let live = Arc::new(Mutex::new(Some(group)));
        let reaped = Arc::clone(&live);
        let waker = self.waker.clone();
        let waiting = thread::Builder::new()
            .name("neotmux-job-wait".into())
            .spawn(move || {
                // Both readers first: the consumer stops reading a job at its ending, so an
                // exit reported before the pipes were drained would lose the tail of the
                // output. A child that exits while a grandchild still holds the pipe open is
                // reported when the pipe closes, which is when the last of its output is in.
                for reader in readers {
                    let _ = reader.join();
                }
                let chunk = match child.wait() {
                    Ok(status) => JobChunk::Exited(code(&status)),
                    Err(err) => JobChunk::Failed(format!("could not wait for the job: {err}")),
                };
                *reaped.lock().unwrap_or_else(PoisonError::into_inner) = None;
                let _ = tx.send(chunk);
                waker.wake();
            });
        if let Err(err) = waiting {
            // The child went into the closure that could not be spawned, so it is reaped by
            // pid here rather than through a `Child` nobody holds any more.
            end_group(group);
            let _ = nix::sys::wait::waitpid(group, None);
            return Err(err);
        }

        Ok(Box::new(Process { chunks, live }))
    }
}

/// Ends a job that will never be handed over: the group first, then the child itself, and
/// then the wait that keeps it from becoming a zombie nobody is left to reap.
fn abandon(mut child: std::process::Child, group: Pid) {
    end_group(group);
    let _ = child.wait();
}

/// SIGKILL to the whole group and to the child by name. The group is what reaches whatever
/// the program started; the child by name is what reaches it in the moment between the fork
/// and the `setpgid` the group is made by, where the group does not exist yet.
fn end_group(group: Pid) {
    let _ = killpg(group, Signal::SIGKILL);
    let _ = kill(group, Signal::SIGKILL);
}

/// One pipe's reader. Every chunk is followed by a nudge, because a channel is not something
/// the frame loop can wait on — the same reason `neotmux-pty`'s `read_loop` gives.
fn reader<R: Read + Send + 'static>(
    name: &str,
    mut pipe: R,
    wrap: fn(Vec<u8>) -> JobChunk,
    tx: &SyncSender<JobChunk>,
    waker: &Waker,
) -> io::Result<JoinHandle<()>> {
    let tx = tx.clone();
    let waker = waker.clone();
    let name = name.to_string();
    thread::Builder::new()
        .name("neotmux-job-reader".into())
        .spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => {
                        if tx.send(wrap(buf[..n].to_vec())).is_err() {
                            return;
                        }
                        waker.wake();
                    }
                    Err(err) => {
                        let _ = tx.send(JobChunk::Failed(format!("reading {name}: {err}")));
                        waker.wake();
                        return;
                    }
                }
            }
        })
}

/// What the plugin is told the program exited with. A child killed by a signal has no code of
/// its own, and the shell's answer for it — 128 plus the signal — is the one people know.
fn code(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt as _;
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(-1)
}

/// A job that has started: what its threads have said, and the pid to signal while it lives.
struct Process {
    chunks: Receiver<JobChunk>,
    /// The job's process group — the child's own pid — or `None` once the waiting thread has
    /// reaped it, so a kill after the fact does not signal whatever the system handed out next.
    live: Arc<Mutex<Option<Pid>>>,
}

impl Running for Process {
    fn poll(&mut self) -> Vec<JobChunk> {
        let mut out = Vec::new();
        while let Ok(chunk) = self.chunks.try_recv() {
            out.push(chunk);
        }
        out
    }

    fn kill(&mut self) {
        let live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(group) = *live {
            // SIGKILL rather than a hangup and a wait: the caller is inside a frame, and the
            // only thing it can afford is the signal itself. The group is cleared by the
            // thread that reaps the child, so a second kill — or one after the job already
            // ended — signals nothing.
            end_group(group);
        }
    }
}

/// Every way a job ends — killed, over its limit, its session gone, the server shutting down —
/// ends with this being dropped, so the signal belongs here for the reason `Shells::drop`
/// hangs its panes up there.
impl Drop for Process {
    fn drop(&mut self) {
        self.kill();
    }
}

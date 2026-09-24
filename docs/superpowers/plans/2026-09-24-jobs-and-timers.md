# Jobs and Timers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A plugin can start an external process and be called back with its output, and can ask
to be called again later.

**Architecture:** The Lua subsystem gains two tables of its own — jobs and timers — and a
`Runner` trait in the shape `Clipboard` already has, so every test drives a fake process. The
host implements `Runner` with `std::process::Command`, reader threads and the server's existing
`Waker`, which is the same path a pane's output already takes. Callbacks are called from inside
the frame, before `dispatch_tick`, with the job's session bound as ambient.

**Tech Stack:** Rust 2024, mlua (Lua 5.4), `std::process`, `std::sync::mpsc::sync_channel`,
`std::thread`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-24-jobs-and-timers-design.md`

## Global Constraints

- `cmd` is a list of strings — program first, then arguments. No string form, no shell.
- `buffered` defaults to `true`. In that mode `on_stdout`/`on_stderr` are never called and
  `on_exit` gets two strings. In streaming mode `on_exit`'s `stdout`/`stderr` are `nil`.
- A streaming callback is handed **complete lines only**; a trailing fragment is held and
  flushed as one last line before `on_exit`.
- `on_exit(code, stdout, stderr, err)` runs **exactly once** for every job that started,
  whatever ended it. `code` is a number and `err` nil on a real exit; `code` is nil and `err` a
  one-line string on every other ending.
- `start` refuses with `(nil, message)`; it does not raise.
- Options: `job_output_bytes` default `1048576`, `job_concurrency` default `8`.
- A callback invocation is one budgeted region, budget `tick_budget_ms`. `tick_overrun_limit`
  consecutive overruns stop a timer / kill a job, and touch nothing else the source owns.
- A timer is clamped up to the frame (16ms) and fires at most once per frame, with no catch-up.
- Jobs and timers belong to the source that started them, and to the session ambient at that
  moment — or, when there was none, to the first session whose frame runs afterwards. Either one
  going away kills the job and stops the timer.
- Job completion never becomes a `neotmux_proto::Event`; nothing here touches `neotmux-proto`.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` are all green before a task is done.
- Comments explain *why*, never *what*. Match the surrounding files' density and voice.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/neotmux-lua/src/runner.rs` (new) | The contract: `JobSpec`, `Runner`, `Running`, `JobChunk`, and `NoRunner` for a host with no way to start one |
| `crates/neotmux-lua/src/jobs.rs` (new) | Bookkeeping for the jobs that are running: ids, owners, sessions, buffers, limits, delivery |
| `crates/neotmux-lua/src/timers.rs` (new) | The timers that are armed, and which of them are due |
| `crates/neotmux-lua/src/api/job.rs` (new) | `neotmux.job.start` and the handle |
| `crates/neotmux-lua/src/api/timer.rs` (new) | `neotmux.timer.every` / `after` and the handle |
| `crates/neotmux-lua/src/api/mod.rs` | `Shared` gains `jobs`, `timers`, `runner`; `install` wires the two tables; the require-failure sweep gains its fifth and sixth lines |
| `crates/neotmux-lua/src/api/opt.rs` | `job_output_bytes`, `job_concurrency` |
| `crates/neotmux-lua/src/lib.rs` | `build_with_runner`, `dispatch_jobs`, `dispatch_timers`, the per-callback budget region, the source sweep |
| `crates/neotmux/src/runner.rs` (new) | The real one: `Command` + `Stdio::piped()` + reader threads + `Waker` |
| `crates/neotmux/src/server.rs` | Build the subsystem with the real runner |
| `crates/neotmux/src/plugins.rs` | Call `dispatch_jobs` / `dispatch_timers` in `respond`, before the tick |
| `crates/neotmux-lua/tests/jobs.rs` (new) | Jobs, against a fake runner |
| `crates/neotmux-lua/tests/timers.rs` (new) | Timers, against an injected clock |
| `crates/neotmux/tests/job_process.rs` (new) | One real process, end to end |
| `docs/plugins.md` | The jobs and timers convention |

---

## Task 1: The runner contract

**Files:**
- Create: `crates/neotmux-lua/src/runner.rs`
- Modify: `crates/neotmux-lua/src/lib.rs` (module declaration, `build_with_runner`, the
  `runner` field), `crates/neotmux-lua/src/api/mod.rs` (`Shared.runner`)
- Test: in `crates/neotmux-lua/src/runner.rs` (unit tests, the way `window.rs` carries its own)

**Interfaces:**
- Consumes: nothing.
- Produces: `runner::{JobSpec, Runner, Running, JobChunk, NoRunner}`;
  `LuaSubsystem::build_with_runner(layers, panes, clipboard, runner)`.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux-lua/src/runner.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A host with no way to start a process says so rather than pretending to have started
    /// one, the same way `clipboard::Absent` answers.
    #[test]
    fn a_host_with_no_runner_refuses_to_start_anything() {
        let spec = JobSpec {
            cmd: vec!["true".to_string()],
            cwd: None,
            env: Vec::new(),
        };
        let err = NoRunner.start(&spec).expect_err("NoRunner started something");
        assert!(err.to_string().contains("cannot start"), "{err}");
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux-lua runner::tests -- --nocapture`
Expected: FAIL — `runner` is not a module yet.

- [ ] **Step 3: Write the contract**

`crates/neotmux-lua/src/runner.rs`:

```rust
//! What the host owes a plugin that wants to run a program.
//!
//! A contract rather than a `Command`, for the same reason [`crate::clipboard::Clipboard`] is
//! one: this crate has no threads and no process of its own, and a test that had to spawn
//! something to check what a plugin does with its output would be testing the operating
//! system. The host implements it; a test hands over a script.

/// What to run. The command is a list because a shell is the caller's to ask for by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpec {
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    /// Added to whatever the host already has, never a replacement: a plugin cannot read the
    /// environment, so it has no way to compose a whole one.
    pub env: Vec<(String, String)>,
}

/// One thing a running job has said. Bytes, not lines: where the line breaks fall is
/// [`crate::jobs`]'s business, and a chunk boundary is not a line boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobChunk {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// The program exited on its own, with this status.
    Exited(i32),
    /// It never got that far: the pipe broke, the child could not be waited for. One line.
    Failed(String),
}

/// A job that has started. Dropping it must not leave a process behind.
pub trait Running {
    /// Whatever has arrived since the last call, and nothing else. Never blocks: this is
    /// called from inside a frame, once per job, and a frame that waited on a child would be
    /// a frame every session waits for.
    fn poll(&mut self) -> Vec<JobChunk>;

    /// Ends it now, without waiting. Idempotent — the caller may already have seen
    /// [`JobChunk::Exited`].
    fn kill(&mut self);
}

/// Somewhere to start a program.
pub trait Runner {
    fn start(&self, spec: &JobSpec) -> std::io::Result<Box<dyn Running>>;
}

/// The one a subsystem built without a host gets: no processes here.
pub struct NoRunner;

impl Runner for NoRunner {
    fn start(&self, _spec: &JobSpec) -> std::io::Result<Box<dyn Running>> {
        Err(std::io::Error::other("this host cannot start processes"))
    }
}
```

- [ ] **Step 4: Wire it into the subsystem**

In `crates/neotmux-lua/src/lib.rs`, add `pub mod runner;` beside `pub mod clipboard;`, add
`runner: Rc<dyn Runner>` to `LuaSubsystem`, and split the constructor the way `build`/`build_with`
already split:

```rust
    pub fn build_with(
        layers: Layers,
        panes: Rc<dyn PaneRead>,
        clipboard: Rc<dyn Clipboard>,
    ) -> anyhow::Result<Self> {
        Self::build_with_runner(layers, panes, clipboard, Rc::new(runner::NoRunner))
    }

    /// The half of `build_with` that takes a way to start a program. A host that has one
    /// hands it over here; every other caller gets `NoRunner`, which refuses.
    pub fn build_with_runner(
        layers: Layers,
        panes: Rc<dyn PaneRead>,
        clipboard: Rc<dyn Clipboard>,
        runner: Rc<dyn runner::Runner>,
    ) -> anyhow::Result<Self> {
        // the existing body of build_with, with `runner` stored on the struct and put into
        // `Shared`
    }
```

Add `pub(crate) runner: Rc<dyn crate::runner::Runner>,` to `Shared` in `api/mod.rs`, destructure
it in `install`, and bind it to `_runner` for now so the crate still compiles — Task 2 uses it.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux-lua && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "a contract for starting a program"
```

---

## Task 2: Jobs that run and finish

**Files:**
- Create: `crates/neotmux-lua/src/jobs.rs`, `crates/neotmux-lua/src/api/job.rs`,
  `crates/neotmux-lua/tests/jobs.rs`
- Modify: `crates/neotmux-lua/src/lib.rs` (`jobs` field, `dispatch_jobs`),
  `crates/neotmux-lua/src/api/mod.rs` (`Shared.jobs`, install `neotmux.job`),
  `crates/neotmux-lua/src/api/opt.rs` (`job_concurrency`)
- Test: `crates/neotmux-lua/tests/jobs.rs`

**Interfaces:**
- Consumes: `runner::{JobSpec, Runner, Running, JobChunk}` from Task 1.
- Produces: `jobs::{Jobs, JobId}`; `Jobs::start(...) -> Result<JobId, String>`,
  `Jobs::kill(JobId) -> bool`, `Jobs::deliver(session) -> Vec<Delivery>`;
  `LuaSubsystem::dispatch_jobs(&self, session: SessionId)`; the Lua table `neotmux.job`.

Buffered mode only. Streaming is Task 3, limits and budget are Tasks 3 and 5.

- [ ] **Step 1: Write the failing test**

`crates/neotmux-lua/tests/jobs.rs` — the harness plus the first two tests. The fake runner is
the point: it hands back a script with no process and no timing.

```rust
//! `neotmux.job`: a plugin runs a program and is called back with what it said. Every test
//! here drives a fake `Runner`, so nothing spawns and nothing waits — what a real process adds
//! is tested once, in `crates/neotmux/tests/job_process.rs`.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neotmux_lua::ops::Op;
use neotmux_lua::runner::{JobChunk, JobSpec, Runner, Running};
use neotmux_lua::runtime::Layers;
use neotmux_lua::LuaSubsystem;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 10, cols: 40 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// What one fake job will say, in order, one `poll` at a time: `script[0]` is the first
/// poll's answer, `script[1]` the second's. An empty vector is "nothing yet".
#[derive(Default)]
struct Script(Vec<Vec<JobChunk>>);

struct FakeRunning {
    script: std::vec::IntoIter<Vec<JobChunk>>,
    killed: Rc<RefCell<bool>>,
}

impl Running for FakeRunning {
    fn poll(&mut self) -> Vec<JobChunk> {
        self.script.next().unwrap_or_default()
    }

    fn kill(&mut self) {
        *self.killed.borrow_mut() = true;
    }
}

/// Answers every `start` with the next script, and records what it was asked to run.
struct FakeRunner {
    scripts: RefCell<std::collections::VecDeque<Script>>,
    started: RefCell<Vec<JobSpec>>,
    killed: Rc<RefCell<bool>>,
    refuse: RefCell<Option<String>>,
}

impl FakeRunner {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            scripts: RefCell::new(std::collections::VecDeque::new()),
            started: RefCell::new(Vec::new()),
            killed: Rc::new(RefCell::new(false)),
            refuse: RefCell::new(None),
        })
    }

    fn will_say(&self, polls: Vec<Vec<JobChunk>>) {
        self.scripts.borrow_mut().push_back(Script(polls));
    }
}

impl Runner for FakeRunner {
    fn start(&self, spec: &JobSpec) -> std::io::Result<Box<dyn Running>> {
        if let Some(why) = self.refuse.borrow().clone() {
            return Err(std::io::Error::other(why));
        }
        self.started.borrow_mut().push(spec.clone());
        let script = self.scripts.borrow_mut().pop_front().unwrap_or_default();
        Ok(Box::new(FakeRunning {
            script: script.0.into_iter(),
            killed: Rc::clone(&self.killed),
        }))
    }
}

fn built(extra: &str, runner: Rc<FakeRunner>) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'sync', 'hints' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        Rc::new(neotmux_lua::clipboard::Absent),
        runner,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
    (dir, Built::new(sub, terminals))
}

/// Starts a job when `J` is pressed. Most tests here start from a key rather than from the
/// `init.lua`, so that the session a job's callbacks run in is the obvious one rather than the
/// adoption rule's.
fn on_key(body: &str) -> String {
    format!(
        "neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
         {body}\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    )
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        sub.dispatch_key(support::one(), spec);
    }
}

/// One frame: deliver whatever the jobs have said, then tick.
fn frame(sub: &LuaSubsystem) {
    sub.dispatch_jobs(support::one());
    sub.dispatch_tick(support::one());
}

fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A job that runs and stops hands its whole output to `on_exit`, unsplit.
#[test]
fn a_buffered_job_answers_with_everything_it_said() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"one\ntw".to_vec())],
        vec![JobChunk::Stdout(b"o\n".to_vec()), JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'echo' }, {\n\
                   on_exit = function(code, out, err_text, err)\n\
                     neotmux.log.info('exit code=' .. tostring(code)\n\
                       .. ' out=[' .. tostring(out) .. ']'\n\
                       .. ' err=' .. tostring(err))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert!(logged(&sub).is_empty(), "on_exit ran before the job ended");

    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line == "exit code=0 out=[one\ntwo\n] err=nil"),
        "{said:?}"
    );
}

/// The command is a list, and it reaches the runner as one.
#[test]
fn the_command_and_its_arguments_reach_the_runner_as_a_list() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &on_key("    neotmux.job.start({ 'git', 'status', '--porcelain' }, { cwd = '/tmp' })"),
        Rc::clone(&runner),
    );
    keys(&sub, &["J"]);

    let started = runner.started.borrow();
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0].cmd, vec!["git", "status", "--porcelain"]);
    assert_eq!(started[0].cwd.as_deref(), Some("/tmp"));
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test jobs`
Expected: FAIL — `neotmux.job` is nil, `dispatch_jobs` does not exist.

- [ ] **Step 3: Write the bookkeeping**

`crates/neotmux-lua/src/jobs.rs`:

```rust
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

use crate::runner::{JobSpec, Runner, Running};
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
    /// Consecutive budget overruns in this job's own callbacks.
    overruns: u32,
}

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
        session: SessionId,
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
                overruns: 0,
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
}
```

The rest of `Jobs` — `deliver`, the line splitting, the limits — lands in Task 3. For Task 2,
add only what the two tests need:

```rust
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
                    crate::runner::JobChunk::Stdout(bytes) => job.stdout.extend_from_slice(&bytes),
                    crate::runner::JobChunk::Stderr(bytes) => job.stderr.extend_from_slice(&bytes),
                    crate::runner::JobChunk::Exited(code) => {
                        finished.push((*id, Some(code), None));
                        break;
                    }
                    crate::runner::JobChunk::Failed(why) => {
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
```

- [ ] **Step 4: Write the binding**

`crates/neotmux-lua/src/api/job.rs` installs `neotmux.job` with one function. `start` reads the
options table, mints the id through `Jobs::start`, and answers a handle table the way
`api/float.rs` does — `id`, `kill`:

```rust
//! `neotmux.job`: run a program, hear what it said.

use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::ambient::Ambient;
use crate::api::opt::Opts;
use crate::jobs::{Callbacks, JobId, Jobs};
use crate::runner::JobSpec;
use crate::source;
use crate::source::Sources;

pub(crate) fn install(
    lua: &Lua,
    jobs: Rc<Jobs>,
    sources: Rc<Sources>,
    ambient: Rc<Ambient>,
    opts: Rc<std::cell::RefCell<Opts>>,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let start = lua.create_function(move |lua, (cmd, options): (Table, Option<Table>)| {
        let mut argv = Vec::new();
        for value in cmd.sequence_values::<String>() {
            argv.push(value?);
        }
        if argv.is_empty() {
            return Ok((Value::Nil, Value::String(lua.create_string("a job needs a command")?)));
        }
        let options = match options {
            Some(table) => table,
            None => lua.create_table()?,
        };
        // None while an `init.lua` is running, which is where a plugin most naturally starts
        // its work: the job is adopted by the first session to run a frame (see `Jobs`).
        let session = ambient.current();
        let spec = JobSpec {
            cmd: argv,
            cwd: options.get::<Option<String>>("cwd")?,
            env: match options.get::<Option<Table>>("env")? {
                Some(env) => {
                    let mut pairs = Vec::new();
                    for entry in env.pairs::<String, String>() {
                        let (key, value) = entry?;
                        pairs.push((key, value));
                    }
                    pairs.sort();
                    pairs
                }
                None => Vec::new(),
            },
        };
        let buffered = options.get::<Option<bool>>("buffered")?.unwrap_or(true);
        let keep = |name: &str| -> mlua::Result<Option<Rc<mlua::RegistryKey>>> {
            Ok(match options.get::<Option<Function>>(name)? {
                Some(f) => Some(Rc::new(lua.create_registry_value(f)?)),
                None => None,
            })
        };
        let callbacks = Callbacks {
            on_stdout: keep("on_stdout")?,
            on_stderr: keep("on_stderr")?,
            on_exit: keep("on_exit")?,
        };
        let owner = source::caller(lua, &sources);
        let concurrency = opts.borrow().job_concurrency;
        match jobs.start(spec, owner, session, buffered, callbacks, concurrency) {
            Ok(id) => Ok((Value::Table(handle(lua, Rc::clone(&jobs), id)?), Value::Nil)),
            Err(why) => Ok((Value::Nil, Value::String(lua.create_string(&why)?))),
        }
    })?;
    table.set("start", start)?;
    Ok(table)
}

fn handle(lua: &Lua, jobs: Rc<Jobs>, id: JobId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", id.0)?;
    let kill = lua.create_function(move |_, _self: Value| Ok(jobs.kill(id)))?;
    handle.set("kill", kill)?;
    Ok(handle)
}
```

Add `job_concurrency: usize` to `Opts` (default `8`) and its `newindex` arm in
`api/opt.rs`, beside `tick_overrun_limit`. Add `jobs: Rc<Jobs>` to `Shared`, build it in
`build_with_runner` as `Rc::new(Jobs::new(runner.clone()))`, and in `install` do
`neotmux.set("job", job::install(lua, jobs.clone(), sources.clone(), ambient.clone(), opts.clone())?)?`.

- [ ] **Step 5: Write the dispatch**

In `lib.rs`, beside `dispatch_tick`:

```rust
    /// What this session's jobs have to say, before the tick that will draw it. Each callback
    /// is its own budgeted region, and the session is bound so that an op a callback raises
    /// reaches the frame that is running.
    pub fn dispatch_jobs(&self, session: SessionId) {
        let deliveries = self.jobs.deliver(session);
        if deliveries.is_empty() {
            return;
        }
        let _bound = self.ambient.bind(session);
        for delivery in deliveries {
            self.run_delivery(delivery);
        }
    }
```

`run_delivery` calls the kept `RegistryKey` with the delivery's arguments — `Exit` passes
`(code, stdout, stderr, err)`, each `nil` where it is `None` — reporting a raising callback
through the existing `report` path, attributed to the delivery's `owner`. Task 5 adds the
budget region around each call; Task 2 calls them directly.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p neotmux-lua --test jobs`
Expected: PASS, both tests.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a plugin can run a program and hear what it said"
```

---

## Task 3: Streaming, kill, and the output limit

**Files:**
- Modify: `crates/neotmux-lua/src/jobs.rs`, `crates/neotmux-lua/src/api/opt.rs`
- Test: `crates/neotmux-lua/tests/jobs.rs`

**Interfaces:**
- Consumes: `Jobs`, `Delivery`, `JobChunk` from Task 2.
- Produces: streaming deliveries (`Delivery::Lines`), `Jobs::kill` leaving an `Exit` behind,
  `Opts::job_output_bytes`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/neotmux-lua/tests/jobs.rs`:

```rust
/// A streaming job is handed complete lines and nothing else: a chunk that stops mid-line is
/// held until the rest of it arrives, and what is still held at the end is flushed as one
/// last line. A plugin never joins a fragment to its remainder because it is never given one.
#[test]
fn a_streaming_job_is_handed_whole_lines_and_the_last_fragment_is_flushed() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"alpha\nbra".to_vec())],
        vec![JobChunk::Stdout(b"vo\ncharlie".to_vec())],
        vec![JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'tail' }, {\n\
                   buffered = false,\n\
                   on_stdout = function(lines)\n\
                     neotmux.log.info('lines=[' .. table.concat(lines, '|') .. ']')\n\
                   end,\n\
                   on_exit = function(code, out)\n\
                     neotmux.log.info('exit out=' .. tostring(out))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert_eq!(logged(&sub), vec!["lines=[alpha]"]);

    frame(&sub);
    assert_eq!(logged(&sub), vec!["lines=[bravo]"]);

    frame(&sub);
    let said = logged(&sub);
    assert_eq!(
        said,
        vec!["lines=[charlie]", "exit out=nil"],
        "the held fragment was not flushed before on_exit, or on_exit kept a copy"
    );
}

/// `kill` ends it, and the caller hears the end of what it started: `code` nil, `err` set.
#[test]
fn killing_a_job_still_calls_on_exit_once() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"working\n".to_vec())]]);
    let (_dir, sub) = built(
        "local job\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             job = neotmux.job.start({ 'sleep' }, {\n\
               on_exit = function(code, out, errout, err)\n\
                 neotmux.log.info('exit code=' .. tostring(code) .. ' err=' .. tostring(err))\n\
               end,\n\
             })\n\
             return true\n\
           end\n\
           if key == 'K' then\n\
             neotmux.log.info('killed=' .. tostring(job:kill()))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    sub.dispatch_key(support::one(), "K");
    frame(&sub);
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "killed=true"), "{said:?}");
    assert_eq!(
        said.iter()
            .filter(|line| line.starts_with("exit code=nil err=killed"))
            .count(),
        1,
        "on_exit did not run exactly once for a killed job: {said:?}"
    );

    // A second kill is not a second ending.
    sub.dispatch_key(support::one(), "K");
    frame(&sub);
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "killed=false"), "{said:?}");
    assert!(
        !said.iter().any(|line| line.starts_with("exit ")),
        "a dead job ended twice: {said:?}"
    );
}

/// A job that says more than it is allowed to is killed, and says why.
#[test]
fn a_job_that_overruns_the_output_limit_is_killed_and_says_so() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(vec![b'x'; 64])]]);
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.job_output_bytes = 16\n{}",
            on_key(
                "    neotmux.job.start({ 'yes' }, {\n\
                       on_exit = function(code, out, errout, err)\n\
                         neotmux.log.info('exit code=' .. tostring(code)\n\
                           .. ' err=' .. tostring(err)\n\
                           .. ' kept=' .. tostring(out and #out))\n\
                       end,\n\
                     })",
            )
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line.starts_with("exit code=nil err=output limit")),
        "{said:?}"
    );
    assert!(*runner.killed.borrow(), "the job was not killed");
}

/// The ninth job of one source is refused, and refusing is not raising.
#[test]
fn a_source_may_not_have_more_jobs_than_the_limit() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.job_concurrency = 2\n{}",
            on_key(
                "    local kept = {}\n\
                     for i = 1, 3 do\n\
                       local job, why = neotmux.job.start({ 'sleep' }, {})\n\
                       kept[i] = job\n\
                       neotmux.log.info('start ' .. i .. ' = ' .. tostring(job ~= nil)\n\
                         .. ' why=' .. tostring(why))\n\
                     end",
            )
        ),
        Rc::clone(&runner),
    );
    keys(&sub, &["J"]);

    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "start 1 = true why=nil"), "{said:?}");
    assert!(said.iter().any(|l| l == "start 2 = true why=nil"), "{said:?}");
    assert!(
        said.iter()
            .any(|l| l.starts_with("start 3 = false why=too many jobs")),
        "{said:?}"
    );
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test jobs`
Expected: FAIL — streaming delivers nothing, `kill` leaves no `Exit`, no limit.

- [ ] **Step 3: Implement**

In `jobs.rs`:

- Add `held_stdout: Vec<u8>` and `held_stderr: Vec<u8>` to `Job` — what has arrived since the
  last line break, in streaming mode.
- `deliver` in streaming mode: append the chunk to the held buffer, split off every complete
  line (`\n`, and a `\r` immediately before it is part of the break, not of the line), push a
  `Delivery::Lines` when there is at least one, keep the remainder held.
- On the ending chunk, in streaming mode: if anything is still held, push one last
  `Delivery::Lines` with it **before** the `Delivery::Exit`.
- Add `ending: Option<(Option<i32>, Option<String>)>` to `Job` so that `kill` can mark a job as
  ended (`(None, Some("killed".into()))`) and let the next `deliver` produce its `Exit`, which
  is what makes "exactly once" true for every path. `kill` returns `false` for a job already
  marked or already gone.
- Enforce the limit wherever bytes are added: in buffered mode against `stdout.len() +
  stderr.len()`, in streaming mode against the held buffers. Over it, `running.kill()` and mark
  the ending as `(None, Some("output limit".into()))`.
- Add `job_output_bytes: usize` (default `1_048_576`) to `Opts` and its `newindex` arm, and
  pass it into `deliver` (`deliver(&self, session: SessionId, limit: usize)`), the way
  `dispatch_tick` reads `tick_budget_ms` from `opts` each frame.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --test jobs`
Expected: PASS, all six.

- [ ] **Step 5: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a job streams whole lines, ends once, and is bounded"
```

---

## Task 4: Timers

**Files:**
- Create: `crates/neotmux-lua/src/timers.rs`, `crates/neotmux-lua/src/api/timer.rs`,
  `crates/neotmux-lua/tests/timers.rs`
- Modify: `crates/neotmux-lua/src/lib.rs` (`timers` field, `dispatch_timers`),
  `crates/neotmux-lua/src/api/mod.rs` (`Shared.timers`, install `neotmux.timer`)
- Test: `crates/neotmux-lua/tests/timers.rs`

**Interfaces:**
- Consumes: nothing from Tasks 1–3 except the dispatch pattern.
- Produces: `timers::{Timers, TimerId}`; `Timers::every(ms, owner, session, callback) -> TimerId`,
  `Timers::after(...)`, `Timers::stop(TimerId) -> bool`, `Timers::due(session, now) -> Vec<Due>`;
  `LuaSubsystem::dispatch_timers(&self, session: SessionId, now: Instant)`.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/timers.rs` (the harness is `jobs.rs`'s, minus the runner — build with
`LuaSubsystem::build`, and drive frames with an `Instant` the test moves):

```rust
//! `neotmux.timer`: a plugin asks to be called again later. The clock is the test's — every
//! frame is handed the instant it happens at, the way `PaneInfoCache::refresh` already is.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::LuaSubsystem;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 10, cols: 40 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// The shipped layer with every plugin off, so a frame holds nothing but the test's own
/// timers. No runner: nothing here starts a process.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'sync', 'hints' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
    (dir, Built::new(sub, terminals))
}

fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn frame(sub: &LuaSubsystem, at: Instant) {
    sub.dispatch_timers(support::one(), at);
    sub.dispatch_tick(support::one());
}

/// `every` fires each time its interval has passed, and goes on firing.
#[test]
fn every_fires_once_its_interval_has_passed_and_keeps_firing() {
    let (_dir, sub) = built(
        "neotmux.timer.every(100, function() neotmux.log.info('fired') end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(16));
    assert!(logged(&sub).is_empty(), "fired before its interval");

    frame(&sub, start + Duration::from_millis(120));
    assert_eq!(logged(&sub), vec!["fired"]);

    frame(&sub, start + Duration::from_millis(180));
    assert!(logged(&sub).is_empty(), "fired twice in one interval");

    frame(&sub, start + Duration::from_millis(240));
    assert_eq!(logged(&sub), vec!["fired"]);
}

/// A timer armed while an `init.lua` runs — before any session exists — is adopted by the
/// first session to run a frame, and fires there. This is the ordinary case for a plugin that
/// arms a poll at load, so it is not left to the other tests to cover by accident.
#[test]
fn a_timer_armed_before_any_session_exists_fires_in_the_first_frame_that_comes() {
    let (_dir, sub) = built(
        "neotmux.timer.every(10, function() neotmux.log.info('fired') end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(50));
    assert_eq!(logged(&sub), vec!["fired"]);
}

/// A timer that should have fired five times while nothing was ticking fires once. There is
/// no catch-up: a plugin that cares how much time passed asks the clock, and one that does
/// not would rather be called once than five times in a row.
#[test]
fn a_timer_that_missed_several_intervals_fires_once() {
    let (_dir, sub) = built(
        "neotmux.timer.every(100, function() neotmux.log.info('fired') end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(1000));
    assert_eq!(logged(&sub), vec!["fired"]);
    frame(&sub, start + Duration::from_millis(1016));
    assert!(logged(&sub).is_empty(), "the missed intervals were caught up");
}

/// `after` fires once and is then done, whatever happens next.
#[test]
fn after_fires_once_and_stops() {
    let (_dir, sub) = built(
        "local t = neotmux.timer.after(50, function() neotmux.log.info('fired') end)\n\
         neotmux.on_key(function(key)\n\
           if key == 'S' then\n\
             neotmux.log.info('stop=' .. tostring(t:stop()))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(60));
    assert_eq!(logged(&sub), vec!["fired"]);

    frame(&sub, start + Duration::from_millis(200));
    assert!(logged(&sub).is_empty(), "a one-shot fired twice");

    sub.dispatch_key(support::one(), "S");
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "stop=false"),
        "stopping a timer that already fired claimed to stop something: {said:?}"
    );
}

/// A timer stopped from inside its own callback does not fire again.
#[test]
fn a_timer_can_stop_itself() {
    let (_dir, sub) = built(
        "local t\n\
         t = neotmux.timer.every(10, function()\n\
           neotmux.log.info('fired')\n\
           t:stop()\n\
         end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(20));
    assert_eq!(logged(&sub), vec!["fired"]);
    frame(&sub, start + Duration::from_millis(400));
    assert!(logged(&sub).is_empty(), "a timer that stopped itself fired again");
}

/// An interval below a frame is a frame: asking for 1ms means "every frame", and is not a
/// promise the host could keep any other way.
#[test]
fn an_interval_below_a_frame_fires_every_frame() {
    let (_dir, sub) = built(
        "neotmux.timer.every(1, function() neotmux.log.info('fired') end)\n",
    );
    let start = Instant::now();

    frame(&sub, start + Duration::from_millis(16));
    assert_eq!(logged(&sub), vec!["fired"]);
    frame(&sub, start + Duration::from_millis(32));
    assert_eq!(logged(&sub), vec!["fired"]);
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test timers`
Expected: FAIL — `neotmux.timer` is nil.

- [ ] **Step 3: Write the timers**

`crates/neotmux-lua/src/timers.rs`:

```rust
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerId(pub u64);

struct Timer {
    owner: SourceId,
    /// None only between a load-time arm and the first frame, which adopts it.
    session: Option<SessionId>,
    interval: Duration,
    /// None for a one-shot, which is dropped the moment it fires.
    repeating: bool,
    due_at: Instant,
    callback: Rc<RegistryKey>,
    overruns: u32,
}

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
```

with `arm(ms, repeating, owner, session, callback, now) -> TimerId` (clamping `ms` up to
`FRAME`), `stop(id) -> bool` (idempotent, false for one already fired or stopped), and:

```rust
    /// The timers of `session` that are due at `now`, and the bookkeeping that goes with
    /// having fired them: a repeating one's next deadline is `now + interval` — not
    /// `due_at + interval`, which is what would produce the catch-up burst — and a one-shot
    /// is gone.
    pub fn due(&self, session: SessionId, now: Instant) -> Vec<Due> {
        let mut out = Vec::new();
        let mut spent = Vec::new();
        for (id, timer) in self.armed.borrow_mut().iter_mut() {
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
                spent.push(*id);
            }
        }
        let mut armed = self.armed.borrow_mut();
        for id in spent {
            armed.remove(&id);
        }
        out
    }
```

`api/timer.rs` installs `neotmux.timer` with `every` and `after`, both taking `(ms, function)`,
both answering a handle table with `stop`, attributed with `source::caller` and with
`ambient.current()` — which is `None` during a load, and is then filled in by the first session
that runs a frame, the same adoption `Jobs` does. `dispatch_timers(session,
now)` in `lib.rs` mirrors `dispatch_jobs`: bind the session, run each due callback, report a
raising one against its owner.

A timer stopped from inside its own callback must not fire again in the same pass: `due` takes
its list before any callback runs, so `dispatch_timers` checks `timers.is_armed(id)` before
each call.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --test timers`
Expected: PASS, all five.

- [ ] **Step 5: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a plugin can ask to be called again later"
```

---

## Task 5: Budget, overrun and lifetime

**Files:**
- Modify: `crates/neotmux-lua/src/lib.rs` (the budget region around each callback, the source
  sweep), `crates/neotmux-lua/src/jobs.rs`, `crates/neotmux-lua/src/timers.rs`,
  `crates/neotmux-lua/src/api/mod.rs` (the require-failure sweep)
- Test: `crates/neotmux-lua/tests/jobs.rs`, `crates/neotmux-lua/tests/timers.rs`

**Interfaces:**
- Consumes: everything from Tasks 2–4.
- Produces: `Jobs::remove_all_of(owner)`, `Jobs::remove_all_for(session)`,
  `Timers::remove_all_of(owner)`, `Timers::remove_all_for(session)`, and the overrun counters.

- [ ] **Step 1: Write the failing tests**

In `tests/timers.rs`:

```rust
/// A timer whose callback keeps overrunning is stopped — and only it. The source's drawing is
/// not the thing that misbehaved, so the source's drawing is left alone.
#[test]
fn a_timer_that_keeps_overrunning_is_stopped_and_the_tick_is_not() {
    let (_dir, sub) = built(
        "neotmux.opt.tick_budget_ms = 1\n\
         neotmux.opt.tick_overrun_limit = 2\n\
         neotmux.timer.every(10, function()\n\
           neotmux.log.info('timer')\n\
           local t = os.clock()\n\
           while os.clock() - t < 0.05 do end\n\
         end)\n\
         neotmux.on_tick(function() neotmux.log.info('tick') end)\n",
    );
    let start = Instant::now();

    for step in 1..=4 {
        frame(&sub, start + Duration::from_millis(100 * step));
    }
    let said = logged(&sub);
    assert_eq!(
        said.iter().filter(|line| *line == "timer").count(),
        2,
        "the timer was not stopped after two overruns: {said:?}"
    );
    assert!(
        said.iter().filter(|line| *line == "tick").count() >= 4,
        "stopping the timer took the source's tick with it: {said:?}"
    );
}
```

In `tests/jobs.rs`:

```rust
/// A raising callback is reported and the job keeps running: the next chunk may well be one
/// the plugin handles, and a plugin that wants out calls `kill`.
#[test]
fn a_raising_callback_does_not_end_the_job() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"one\n".to_vec())],
        vec![JobChunk::Stdout(b"two\n".to_vec())],
        vec![JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'tail' }, {\n\
                   buffered = false,\n\
                   on_stdout = function(lines)\n\
                     neotmux.log.info('saw ' .. lines[1])\n\
                     error('boom')\n\
                   end,\n\
                   on_exit = function() neotmux.log.info('exited') end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    frame(&sub);
    frame(&sub);
    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "saw one"), "{said:?}");
    assert!(said.iter().any(|l| l == "saw two"), "{said:?}");
    assert!(said.iter().any(|l| l == "exited"), "{said:?}");
}

/// A plugin whose load fails takes its jobs with it: the callbacks belong to a chunk that is
/// not there, so nothing is left that could be told how the job ended. The require-failure
/// sweep does it — the same sweep that drops registrations, reservations and floats — and the
/// require is reached from a key, because a `require` during load has no session to start a
/// job in.
#[test]
fn a_require_that_fails_takes_its_sources_jobs_with_it() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"working\n".to_vec())]]);
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lua/plugin")).unwrap();
    fs::write(
        dir.path().join("lua/plugin/worker.lua"),
        "neotmux.job.start({ 'sleep' }, {})\nerror('nope')\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "local defaults = require('neotmux.defaults')\n\
         for _, name in ipairs({ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
           'notify', 'borders', 'sync', 'hints' }) do\n\
           defaults[name] = false\n\
         end\n\
         neotmux.on_key(function(key)\n\
           if key == 'R' then\n\
             require('plugin.worker')\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    )
    .unwrap();

    let terminals = support::panes();
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        Rc::new(neotmux_lua::clipboard::Absent),
        Rc::clone(&runner) as Rc<dyn Runner>,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
    let sub = Built::new(sub, terminals);

    keys(&sub, &["R"]);
    assert_eq!(runner.started.borrow().len(), 1, "the job never started");
    assert!(
        *runner.killed.borrow(),
        "the failed require left its source's job running"
    );
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test timers --test jobs`
Expected: FAIL — no budget region, no sweep.

- [ ] **Step 3: Implement the budget region**

In `lib.rs`, one helper both dispatches use — it arms the hook once for the batch and enters a
region per callback, because a region is per callback but arming is per pass:

```rust
    /// Runs one job or timer callback inside its own budgeted region and says whether it
    /// overran. Arming is the caller's, once for the whole batch: `Budget::arm` costs the
    /// interpreter a hook, and doing it per callback would cost it per callback.
    fn measured_callback(&self, owner: SourceId, call: impl FnOnce()) -> bool {
        let budget_ms = self.opts.borrow().tick_budget_ms;
        self.budget.enter(Duration::from_millis(budget_ms));
        call();
        let outcome = self.budget.leave();
        if outcome != Outcome::WithinBudget {
            self.report_text(
                owner,
                format!("a callback exceeded the {budget_ms} ms budget"),
            );
        }
        overrun::counts_toward_park(outcome, Duration::from_millis(budget_ms))
    }
```

`dispatch_jobs` and `dispatch_timers` arm the budget at the top, disarm at the bottom, and count
per job / per timer: reaching `tick_overrun_limit` consecutively kills the job (with
`err = "budget"`) or stops the timer, and says so with a notify. A callback that stays within
budget resets its own counter to zero. Neither touches `Overrun::park`, which is the tick's.

- [ ] **Step 4: Implement the lifetime rules**

- `Jobs::remove_all_of(owner)` kills and drops every job of that source, **without** an `Exit`
  delivery — the callback that would receive it belongs to the chunk that is going away.
  `Timers::remove_all_of(owner)` likewise.
- Call both in `api/mod.rs`'s require-failure sweep, beside `floats.remove_all_of(id)`, and
  anywhere else `registry.remove_all_of` is called.
- `Jobs::remove_all_for(session)` / `Timers::remove_all_for(session)` for a session that is
  gone; call them from wherever the host tells the subsystem a session ended
  (`LuaSubsystem::forget_session`, adding it if there is none — grep for `ambient` cleanup).

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux-lua`
Expected: PASS.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a runaway callback stops the thing that keeps calling it"
```

---

## Task 6: The real runner, and the documentation

**Files:**
- Create: `crates/neotmux/src/runner.rs`, `crates/neotmux/tests/job_process.rs`
- Modify: `crates/neotmux/src/lib.rs` or `main.rs` (module), `crates/neotmux/src/server.rs`
  (build with the runner), `crates/neotmux/src/plugins.rs` (`respond` calls the two dispatches),
  `docs/plugins.md`
- Test: `crates/neotmux/tests/job_process.rs`

**Interfaces:**
- Consumes: `neotmux_lua::runner::{Runner, Running, JobSpec, JobChunk}`.
- Produces: `crate::runner::Processes` — a `Runner` backed by `std::process::Command`.

- [ ] **Step 1: Write the failing test**

`crates/neotmux/tests/job_process.rs`:

```rust
//! One real process, to prove the seam is wired to something that exists. Everything about
//! what a plugin sees is in `neotmux-lua`'s own tests, against a fake.

use std::time::{Duration, Instant};

use neotmux_lua::runner::{JobChunk, JobSpec, Runner};

#[test]
fn a_real_process_runs_and_its_output_comes_back() {
    let (wakeup, waker) = neotmux_term::wakeup::wakeup().unwrap();
    let runner = neotmux::runner::Processes::new(waker);
    let mut running = runner
        .start(&JobSpec {
            cmd: vec!["/bin/echo".into(), "hello".into()],
            cwd: None,
            env: Vec::new(),
        })
        .expect("/bin/echo did not start");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    let mut code = None;
    while Instant::now() < deadline && code.is_none() {
        for chunk in running.poll() {
            match chunk {
                JobChunk::Stdout(bytes) => out.extend_from_slice(&bytes),
                JobChunk::Exited(status) => code = Some(status),
                JobChunk::Failed(why) => panic!("{why}"),
                JobChunk::Stderr(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(code, Some(0), "/bin/echo did not exit");
    assert_eq!(String::from_utf8_lossy(&out).trim(), "hello");
    drop(wakeup);
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --test job_process`
Expected: FAIL — `neotmux::runner` does not exist.

- [ ] **Step 3: Write the runner**

`crates/neotmux/src/runner.rs`: `Processes { waker: Waker }`, whose `start`:

- builds a `std::process::Command` from `spec.cmd[0]` and the rest, sets `current_dir` from
  `cwd` and `envs` from `spec.env` (added, never `env_clear`), and
  `.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())`;
- spawns one reader thread per pipe, each sending down a `sync_channel(64)` and calling
  `waker.wake()` after every send — the shape `neotmux-term`'s `read_loop` already has, and
  the reason a job needs no file descriptor in the poll set;
- spawns a third thread that `wait`s for the child and sends `JobChunk::Exited(status)` —
  after both readers have seen EOF, so that the last of the output is never delivered after
  the exit;
- `poll` is `try_recv` in a loop until the channel is empty;
- `kill` is `Child::kill` behind a shared handle, ignoring "already exited";
- `Drop` kills the child, the way `Shells`' `Drop` hangs its panes up.

Then in `server.rs`, build the subsystem with it:

```rust
    let runner = Rc::new(crate::runner::Processes::new(serving.waker.clone()));
    let lua = LuaSubsystem::build_with_runner(layers, panes, clipboard, runner)?;
```

and in `plugins.rs`'s `respond`, before the tick and after `catch_up`:

```rust
        // Before the tick, so a tick sees what this frame's jobs and timers have already
        // changed rather than finding out next frame.
        self.lua.dispatch_jobs(session);
        self.lua.dispatch_timers(session, Instant::now());
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p neotmux --test job_process`
Expected: PASS.

- [ ] **Step 5: Write the documentation**

Add a section to `docs/plugins.md` next to the other conventions — "Jobs and timers" — covering:
the two calls with their full option tables; `buffered` and what each mode hands over; that a
streaming callback is given complete lines only; `on_exit`'s four arguments and the rule that it
runs exactly once; that `start` answers `(nil, message)`; the two options and their defaults;
that a timer is clamped to the frame, fires at most once per frame and does not catch up; that a
job and a timer belong to their source and their session and go when either does; that a
callback runs before the tick, in the same frame; and one worked example — a sound on a state
change, using `neotmux.state` — since that is the thing everybody will want first.

Add the plugin count line in that file only if this task ships a plugin; it does not.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "the host runs the program, and the docs say what a plugin gets"
```

---

## Self-review against the spec

| Spec section | Task |
|---|---|
| §2.1 `job.start`, cmd list, cwd, env | 2 |
| §2.1 buffered / streaming, fragment rule, `on_exit` arity and once-only | 2, 3 |
| §2.1 `start` answers `(nil, message)` | 2 (empty cmd, spawn failure), 3 (concurrency) |
| §2.2 `timer.every` / `after` / `stop` | 4 |
| §2.3 options | 2 (`job_concurrency`), 3 (`job_output_bytes`) |
| §3.1 the path bytes take | 6 |
| §3.2 callbacks before the tick, ambient session | 2 (dispatch), 6 (wiring) |
| §3.3 not an `Event` | by construction — nothing in the plan touches `neotmux-proto` |
| §3.4 the id is minted in Lua | 2 (`Jobs::start` mints; the handle carries it) |
| §4 attribution, budget, overrun | 5 |
| §5 time: clamp, once per frame, no catch-up | 4 |
| §6 limits | 3 |
| §7 lifetime: source, session, shutdown | 5 (source, session), 6 (`Drop`) |
| §8 what is said about a failure | 2, 3, 5 |
| §9 testing: two seams, the listed cases, one real process | every task; 6 for the real one |

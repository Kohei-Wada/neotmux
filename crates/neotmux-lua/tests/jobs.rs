//! `neotmux.job`: a plugin runs a program and is called back with what it said. Every test
//! here drives a fake `Runner`, so nothing spawns and nothing waits — what a real process adds
//! is tested once, in `crates/neotmux/tests/job_process.rs`.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runner::{JobChunk, JobSpec, Runner, Running};
use neotmux_lua::runtime::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, SessionId, Size};

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
    begin(&sub, support::one());
    (dir, Built::new(sub, terminals))
}

/// The geometry one session is drawn against. A test that drives more than one session calls
/// it again for the other, since a session with nothing published has no frame to run.
fn begin(sub: &LuaSubsystem, session: SessionId) {
    sub.begin_frame(
        session,
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
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
    logged_in(sub, support::one())
}

fn logged_in(sub: &Built, session: SessionId) -> Vec<String> {
    sub.drain_ops(Some(session))
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

/// The `init.lua` body a job started at load time is written in. It runs before any session
/// exists, so `neotmux.session_name()` is nil there and the job has no session of its own.
fn at_load() -> String {
    "neotmux.job.start({ 'echo' }, {\n\
       on_exit = function(code, out)\n\
         neotmux.log.info('exit code=' .. tostring(code)\n\
           .. ' out=[' .. tostring(out) .. ']')\n\
       end,\n\
     })\n"
        .to_string()
}

/// A job has to be startable from an `init.lua`, which is where a plugin most naturally does
/// its setup, and there is no session bound there. The first session whose frame comes looking
/// adopts it, and that is the session its callbacks run in.
#[test]
fn a_job_started_before_any_session_is_adopted_by_the_first_frame() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(b"loaded\n".to_vec()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built(&at_load(), Rc::clone(&runner));

    assert_eq!(runner.started.borrow().len(), 1, "the load started no job");
    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "exit code=0 out=[loaded\n]"),
        "{said:?}"
    );
}

/// Adoption is a handover, not a broadcast: once a session has picked a job up it is that
/// session's, and every other session's frame walks past it without so much as polling it.
/// Polling matters as much as delivering — a poll from the wrong session would eat the chunk
/// the owner was owed.
#[test]
fn a_session_that_did_not_adopt_a_job_never_sees_it() {
    let two = SessionId::from(2);
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"a\n".to_vec())],
        vec![JobChunk::Stdout(b"b\n".to_vec()), JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(&at_load(), Rc::clone(&runner));
    begin(&sub, two);

    // Session two gets there first, so the job is two's from here on.
    sub.dispatch_jobs(two);
    sub.dispatch_jobs(support::one());
    assert!(
        logged_in(&sub, support::one()).is_empty(),
        "session one heard a job it never adopted"
    );

    // If session one had polled, it would have taken this poll's `Exited` and session two
    // would wait forever; if it had merely delivered, `b` would be missing from the output.
    sub.dispatch_jobs(two);
    let said = logged_in(&sub, two);
    assert!(
        said.iter().any(|line| line == "exit code=0 out=[a\nb\n]"),
        "{said:?}"
    );
    assert!(
        logged_in(&sub, support::one()).is_empty(),
        "session one heard the end of a job it never adopted"
    );
}
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
    assert!(
        said.iter().any(|l| l == "start 1 = true why=nil"),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l == "start 2 = true why=nil"),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("start 3 = false why=too many jobs")),
        "{said:?}"
    );
}

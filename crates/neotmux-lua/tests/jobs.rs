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

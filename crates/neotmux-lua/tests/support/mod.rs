//! What a test has to hold that the host holds in the real thing: the panes the Lua layer
//! reads from. They are not the subsystem's own any more — the session owns them, and from
//! M3 the session is in the other process — so a test that writes into a pane has to keep a
//! handle to it.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neotmux_lua::ops::Op;
use neotmux_lua::runner::{JobChunk, JobSpec, Runner, Running};
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};
use neotmux_term::terminals::PaneTerminals;

/// Panes for a subsystem whose test never writes into one.
pub fn panes() -> Rc<PaneTerminals> {
    Rc::new(PaneTerminals::new())
}

/// A subsystem and the panes behind it. Derefs to the subsystem, so a test reads as though
/// it were holding one.
pub struct Built {
    lua: LuaSubsystem,
    terminals: Rc<PaneTerminals>,
}

impl Built {
    pub fn new(lua: LuaSubsystem, terminals: Rc<PaneTerminals>) -> Self {
        Self { lua, terminals }
    }

    pub fn terminals(&self) -> &PaneTerminals {
        &self.terminals
    }
}

impl Deref for Built {
    type Target = LuaSubsystem;

    fn deref(&self) -> &LuaSubsystem {
        &self.lua
    }
}

/// The session every test in this crate drives. They are about the Lua layer rather than
/// about sessions, and the layer's callers name one for every dispatch.
pub fn one() -> neotmux_proto::SessionId {
    neotmux_proto::SessionId::from(1)
}

/// The size shared by `detect.rs` and `state.rs`'s harness. Other test files pick their own —
/// this one is not a crate-wide default, only the one those two agree on.
pub const SIZE: Size = Size { rows: 16, cols: 40 };

pub fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// The one pane `detect.rs` and `state.rs` both drive.
pub fn pane() -> PaneId {
    PaneId::from(1)
}

/// The shipped runtime with every plugin off. `neotmux.detect` and `neotmux.state` are
/// convention modules — nothing registers them — so the only thing running here is what the
/// test's `init.lua` asks for. Shared by `detect.rs` and `state.rs`, which want the exact same
/// plugins-off list and must not drift apart from each other.
///
/// `chime` is on the list like the rest. It is built with the real job runner, so a test that
/// ticked a pane into `blocked` would spawn a player and `cargo test` would make a noise at
/// whoever ran it; `chime.rs` builds its own runtime and turns it on there.
pub fn built(extra: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'guard', 'sync', 'watch', 'sidebar', 'footer',\n\
               'hints', 'chime' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        one(),
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

/// What the host would have read out of `/proc` for the one pane: the shipped Claude ruleset
/// asks it before any rule runs, and a test that never publishes it is a test about a pane
/// nothing is known about.
pub fn running(sub: &Built, command: &str) {
    sub.publish_pane_info(
        one(),
        std::collections::BTreeMap::from([(
            pane(),
            neotmux_proto::PaneInfo {
                command: Some(command.to_string()),
                cwd: None,
            },
        )]),
    );
}

pub fn feed(sub: &Built, text: &[u8]) {
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), text);
}

pub fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        assert_eq!(
            sub.dispatch_key(one(), spec),
            KeyOutcome::Consumed,
            "the test's own on_key did not take '{spec}'"
        );
    }
}

pub fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// Only the notifications, for a test that counts how often something was said out loud
/// rather than what was logged along the way.
pub fn notified(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// What one fake job will say, in order, one `poll` at a time: `script[0]` is the first
/// poll's answer, `script[1]` the second's. An empty vector is "nothing yet".
#[derive(Default)]
pub struct Script(pub Vec<Vec<JobChunk>>);

pub struct FakeRunning {
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

/// Answers every `start` with the next script, and records what it was asked to run. Shared
/// by `jobs.rs` and `chime.rs`: a plugin that chooses between programs by how they end can
/// only be tested against a runner that decides both whether one starts and how it ends.
pub struct FakeRunner {
    pub scripts: RefCell<VecDeque<Script>>,
    pub started: RefCell<Vec<JobSpec>>,
    pub killed: Rc<RefCell<bool>>,
    refuse: RefCell<Option<String>>,
    refused: RefCell<HashSet<String>>,
}

impl FakeRunner {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            scripts: RefCell::new(VecDeque::new()),
            started: RefCell::new(Vec::new()),
            killed: Rc::new(RefCell::new(false)),
            refuse: RefCell::new(None),
            refused: RefCell::new(HashSet::new()),
        })
    }

    pub fn will_say(&self, polls: Vec<Vec<JobChunk>>) {
        self.scripts.borrow_mut().push_back(Script(polls));
    }

    /// That program is not on this machine: `start` fails for it and for nothing else.
    pub fn refuse_program(&self, name: &str) {
        self.refused.borrow_mut().insert(name.to_string());
    }

    /// Nothing on this machine starts.
    pub fn refuse_all(&self) {
        *self.refuse.borrow_mut() = Some("no such file or directory".to_string());
    }
}

impl Runner for FakeRunner {
    fn start(&self, spec: &JobSpec) -> std::io::Result<Box<dyn Running>> {
        if let Some(why) = self.refuse.borrow().clone() {
            return Err(std::io::Error::other(why));
        }
        if let Some(program) = spec.cmd.first()
            && self.refused.borrow().contains(program)
        {
            return Err(std::io::Error::other("no such file or directory"));
        }
        self.started.borrow_mut().push(spec.clone());
        let script = self.scripts.borrow_mut().pop_front().unwrap_or_default();
        Ok(Box::new(FakeRunning {
            script: script.0.into_iter(),
            killed: Rc::clone(&self.killed),
        }))
    }
}

/// The rule Claude Code draws around its prompt, at the width the captures were taken at.
pub const BAR: &str =
    "────────────────────────────────────────────────────────────────────────────";

/// Captured: idle, nothing running. The prompt is a `❯` between two rules, with the status
/// lines under the lower one.
pub fn idle_screen() -> String {
    format!(
        "✻ Cogitated for 6s · done 9:01 PM\n\
         \n\
         {BAR}\n\
         ❯\n\
         {BAR}\n\
         \x20 ⚠ Transcript saving is off …\n\
         \x20 [Opus 5.5 (1M context)] cmp ●─────────── 7% | ctx …\n\
         \x20 -- INSERT -- ⏵⏵ auto mode on (shift+tab to cycle)\n"
    )
}

/// Captured: a turn running. The same prompt is still on the screen under the spinner line.
pub fn working_screen() -> String {
    format!(
        "\x20 Listing 1 directory… (ctrl+o to expand)\n\
         \x20 ⎿  $ ls -la /etc/hostname\n\
         \n\
         * Actioning… (3s · ↓ 16 tokens)\n\
         \n\
         {BAR}\n\
         ❯\n\
         {BAR}\n"
    )
}

/// Captured: a Bash permission prompt. One rule, and the dialog runs to the bottom of the
/// screen — no second rule, so no prompt box, so nothing reads this as an idle prompt.
pub fn permission_screen() -> String {
    format!(
        "\x20 Reading 1 file… (ctrl+o to expand)\n\
         \x20 ⎿  $ cat /etc/hostname\n\
         \n\
         {BAR}\n\
         \x20Bash command\n\
         \x20Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below\n\
         \n\
         \x20  cat /etc/hostname\n\
         \x20  Print hostname\n\
         \n\
         \x20Do you want to proceed?\n\
         \x20❯ 1. Yes\n\
         \x20  2. Yes, allow reading from /etc from this project\n\
         \x20  3. Yes, and switch to auto mode · auto mode handles these prompts for you\n\
         \x20  4. No\n\
         \n\
         \x20Esc to cancel · Tab to amend\n"
    )
}

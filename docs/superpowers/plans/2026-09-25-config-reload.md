# Config Reload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `config-reload` command that re-runs every layer's `init.lua` in the running server, made safe by nvim-style named groups that empty themselves when named again.

**Architecture:** A host-side `Groups` registry records, for each group, what was registered while a `g.*` wrapper was running (`Held` entries). Naming a group again, or a reload that did not name it, hands those entries to `Sweep`, which already knows how to take back everything a source holds and learns to take back single entries. `neotmux.reload()` raises an `Op::Reload`, which the server turns into a `ServerOp::Reload` and carries out after every session's frame, where key-raised ops are already carried out and the raising client's input is already held.

**Tech Stack:** Rust 2024, mlua 0.12 (Lua 5.4, vendored), stylua + luacheck for `runtime/`.

**Spec:** `docs/superpowers/specs/2026-09-25-config-reload-design.md`

## Global Constraints

- One file = one public type, named after it in `snake_case`; `mod.rs`/`lib.rs` hold only `//!`, `mod` and `use` lines (`docs/superpowers/specs/2026-09-25-layout-one-file-per-type-design.md` §1).
- No new dependencies.
- Comments only where the why is non-obvious; match the surrounding prose style.
- Group names are one namespace across every source; a name that is not a non-empty string raises.
- A job in an emptied group is killed with no `on_exit`; a timer in one is stopped.
- Same-source `keymap.set` / `command.add` replace; a different source is refused as today.
- Verification for every task: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` with no warnings, the named tests; Lua changes also `make lua` (stylua --check) and `make lint` (luacheck).

## Review Focus

1. A reload with copy mode or a hint pick open must leave no pane frozen — `unload` thaws (test in Task 6).
2. A reload whose `init.lua` or one plugin now fails to load must still load the rest, and must not leave the failed plugin's old registrations behind (test in Task 4).
3. Two reloads asked for in one frame must reload once, not twice (test in Task 5).
4. `g.reserve` / `g.float` / `g.job.start` / `g.timer.*` must attribute to the chunk that called `g.*`, not to the host's wrapper — `plugin::caller` skips C frames, but pin it (test in Task 2).
5. A module that names its group lazily (from a callback, not at load) must be swept on a reload that does not name it again, and not error (test in Task 4).

---

### Task 1: Same-source re-registration replaces; `command.remove`

**Files:**
- Modify: `neotmux-lua/src/plugin/commands.rs` (`Commands::add`, new `remove`, `remove_if`)
- Modify: `neotmux-lua/src/api/command.rs` (add `remove` binding; comment on `add`)
- Modify: `neotmux-lua/src/api/keymap.rs:118-140` (replace instead of refuse when the holder is the attempter)
- Test: `neotmux-lua/tests/commands.rs`, `neotmux-lua/tests/keys.rs`

**Interfaces:**
- Produces: `Commands::remove(&self, name: &str) -> bool`; `Commands::remove_if(&self, name: &str, callback: &Rc<RegistryKey>) -> bool` (removes only when the stored callback is `Rc::ptr_eq` to `callback`; Task 2 uses it). Lua: `neotmux.command.remove(name) -> boolean`.

- [ ] **Step 1: Rewrite the existing refusal test to use two sources, and add the replacement tests**

In `neotmux-lua/tests/commands.rs`, replace `a_second_registration_of_a_name_is_refused_and_the_first_holds` with:

```rust
#[test]
fn a_second_registration_from_another_source_is_refused_and_the_first_holds() {
    let (_dir, sub) = built(&[
        (
            "lua/plugin/first.lua",
            "neotmux.command.add('scratch', function() return 'first' end)",
        ),
        (
            "lua/plugin/second.lua",
            "local ok, err = neotmux.command.add('scratch', function() return 'second' end)
             assert(ok == nil, 'the second registration should have been refused')
             neotmux.log.info(err)",
        ),
        ("init.lua", "require('plugin.first') require('plugin.second')"),
    ]);
    assert_eq!(
        sub.run_command(support::one(), "scratch", &[]),
        CommandOutcome::Ran(vec!["first".to_string()])
    );
    let lines = logged(&sub);
    assert!(lines.iter().any(|l| l.contains("already registered")), "{lines:?}");
}

/// A plugin run again registers its commands again, and they are its own: the second one is
/// the one that answers.
#[test]
fn a_second_registration_from_the_same_source_replaces_the_first() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "assert(neotmux.command.add('scratch', function() return 'first' end))
         assert(neotmux.command.add('scratch', function() return 'second' end))",
    )]);
    assert_eq!(
        sub.run_command(support::one(), "scratch", &[]),
        CommandOutcome::Ran(vec!["second".to_string()])
    );
}

#[test]
fn a_removed_command_is_unknown_and_a_second_remove_says_so() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('scratch', function() return 'x' end)
         assert(neotmux.command.remove('scratch') == true)
         assert(neotmux.command.remove('scratch') == false)",
    )]);
    assert!(matches!(
        sub.run_command(support::one(), "scratch", &[]),
        CommandOutcome::Unknown
    ));
}
```

Check `CommandOutcome`'s variant for an unregistered name in `neotmux-lua/src/command_outcome.rs` and use it (the existing `a_name_nobody_registered_is_unknown` test shows it).

In `neotmux-lua/tests/keys.rs`, add:

```rust
/// The same source binding a key it already holds replaces its own binding.
#[test]
fn the_same_source_rebinding_a_key_replaces_its_own_binding() {
    let (_dir, sub) = built(
        "neotmux.keymap.set('C-x', function() neotmux.log.info('first') return true end)
         local h, err = neotmux.keymap.set('C-x', function() neotmux.log.info('second') return true end)
         assert(h ~= nil, tostring(err))",
    );
    sub.dispatch_key(support::one(), "C-x");
    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "second"), "{said:?}");
    assert!(!said.iter().any(|l| l == "first"), "{said:?}");
}
```

Use whatever `built`/`logged` helpers `keys.rs` already defines; if its `built` takes files rather than one init string, pass `&[("init.lua", ...)]`.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p neotmux-lua --test commands --test keys`
Expected: `a_second_registration_from_the_same_source_replaces_the_first` and the keymap test FAIL (refused); the remove test fails to run `remove` (`attempt to call a nil value (field 'remove')`).

- [ ] **Step 3: Implement**

`neotmux-lua/src/plugin/commands.rs`, in `add`:

```rust
        if let Some(existing) = inner.get(&name)
            && existing.owner != owner
        {
            return Err(existing.owner);
        }
```

and update its doc comment: the first registration holds against *another* source; the same source replaces its own. Add:

```rust
    /// Answers whether there was a command of that name. Any source may remove any command:
    /// plugins are trusted, and the one that asks is the one that knows why.
    pub fn remove(&self, name: &str) -> bool {
        self.inner.borrow_mut().remove(name).is_some()
    }

    /// Removes `name` only while it is still the registration `callback` made — a group being
    /// emptied must not take away a command its source has since registered again.
    pub fn remove_if(&self, name: &str, callback: &Rc<RegistryKey>) -> bool {
        let mut inner = self.inner.borrow_mut();
        match inner.get(name) {
            Some(entry) if Rc::ptr_eq(&entry.callback, callback) => {
                inner.remove(name);
                true
            }
            _ => false,
        }
    }
```

`neotmux-lua/src/api/command.rs`: add, beside `add`,

```rust
    {
        let commands = commands.clone();
        let remove = lua.create_function(move |_, name: String| Ok(commands.remove(&name)))?;
        command.set("remove", remove)?;
    }
```

and change the comment on `add` to say the same source replaces.

`neotmux-lua/src/api/keymap.rs`: replace the `if let Some(holder) = holder_of(...)` block with a lookup that compares owners:

```rust
        let attempter = attribute(&sources, &callback);
        if let Some(existing) = find(&registry, &normalised) {
            let held_by = registry.owner(existing);
            if held_by == Some(attempter) {
                registry.remove(existing);
            } else {
                let holder = held_by.and_then(|id| sources.name(id)).unwrap_or_default();
                let attempted_by = sources.name(attempter).unwrap_or_default();
                let message = format!("'{normalised}' is already bound by {holder}");
                ops.push(
                    attempter,
                    Op::Log {
                        level: Level::Warn,
                        text: format!(
                            "keymap conflict on '{normalised}': held by {holder}, refused for {attempted_by}"
                        ),
                    },
                );
                return Ok((Value::Nil, Value::String(lua.create_string(message)?)));
            }
        }
        let key = Rc::new(lua.create_registry_value(callback)?);
        let id = registry.add(attempter, Kind::Keymap(normalised), key);
```

Delete `holder_of` if nothing else uses it.

- [ ] **Step 4: Run to see them pass**

Run: `cargo test -p neotmux-lua --test commands --test keys`
Expected: PASS.

- [ ] **Step 5: Docs for this task**

`docs/plugins.md`, "Registering": in the `command.add` row replace "and there is no `remove`" with "`neotmux.command.remove(name)` takes one away"; after the `keymap.set` refusal paragraph add: "The same source setting a key it already holds replaces its own binding, and the same goes for `command.add`: a plugin run again keeps its keys and commands."

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "a source that registers a key or a command it already holds replaces its own, and command.remove takes one away"
```

---

### Task 2: Groups

**Files:**
- Create: `neotmux-lua/src/plugin/group_id.rs`, `neotmux-lua/src/plugin/held.rs`, `neotmux-lua/src/plugin/groups.rs`, `neotmux-lua/src/plugin/in_group.rs`
- Create: `neotmux-lua/src/api/sweep.rs` (moved out of `api/install.rs`), `neotmux-lua/src/api/group.rs`
- Modify: `neotmux-lua/src/plugin/mod.rs`, `neotmux-lua/src/api/mod.rs`
- Modify: `neotmux-lua/src/api/install.rs` (`Shared` gains `groups`; `register` notes; build `neotmux.group`; `Sweep` moved)
- Modify: `neotmux-lua/src/api/keymap.rs`, `reserve.rs`, `float.rs`, `job.rs`, `timer.rs`, `command.rs` (note what they add)
- Modify: `neotmux-lua/src/sched/jobs.rs` (`Jobs::remove`)
- Modify: `neotmux-lua/src/lua_subsystem.rs` (build `Groups`, pass into `Shared`, keep a field)
- Test: `neotmux-lua/tests/groups.rs` (new)

**Interfaces:**
- Consumes: `Commands::remove_if` (Task 1).
- Produces:
  - `pub struct GroupId(pub u64)` — `Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord`.
  - `pub enum Held { Registration(RegistrationId), Reservation(ReservationId), Float(FloatId), Job(JobId), Timer(TimerId), Command { name: String, callback: Rc<RegistryKey> } }`.
  - `Groups` (`#[derive(Default)]`): `name(&self, name: &str) -> (GroupId, Vec<Held>)`, `enter(&self, id: GroupId) -> InGroup<'_>`, `note(&self, held: Held)`, `begin_reload(&self)`, `end_reload(&self) -> Vec<Held>`.
  - `pub(crate) struct Sweep { registry, buffers, reservations, floats, jobs, timers, commands }` (all `Rc`, `#[derive(Clone)]`) with `remove_all_of(&self, SourceId)` (unchanged behaviour) and `empty(&self, held: Vec<Held>)`.
  - `Jobs::remove(&self, id: JobId)` — kill and drop, no `Exit` delivered.
  - Lua: `neotmux.group(name) -> g`, where `g` carries `on_tick on_event on_key on_write float reserve` and `g.keymap.set`, `g.command.add`, `g.job.start`, `g.timer.every`, `g.timer.after`.

- [ ] **Step 1: Write the failing tests**

Create `neotmux-lua/tests/groups.rs`:

```rust
//! `neotmux.group`: a named set of registrations that empties itself when it is named again.

use std::fs;
use std::rc::Rc;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::{JobChunk, Op};
use neotmux_lua::plugin::{KindTag, Layers};

mod support;

fn built(files: &[(&str, &str)], runner: Rc<dyn neotmux_lua::host::Runner>) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
        Rc::new(neotmux_lua::host::Absent),
        runner,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(None)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn ticks(sub: &LuaSubsystem) -> usize {
    sub.registry().snapshot(KindTag::Tick).len()
}

#[test]
fn naming_a_group_again_empties_it() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "local g = neotmux.group('t')
             g.on_tick(function() end)
             g.on_event('resized', function() end)
             g.keymap.set('C-x', function() return true end)
             g.command.add('scratch', function() end)
             assert(g.reserve('bottom', 1))
             assert(g.float({ x = 0, y = 0, w = 1, h = 1 }))
             g.timer.every(1000, function() end)
             neotmux.group('t')",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert_eq!(ticks(&sub), 0);
    assert!(sub.registry().snapshot(KindTag::Event).is_empty());
    assert!(sub.registry().snapshot(KindTag::Keymap).is_empty());
    assert!(sub.commands().is_empty());
    assert_eq!(sub.reserved(), Default::default());
    assert_eq!(sub.floats().count(), 0);
    assert_eq!(sub.census().timers, 0);
}

#[test]
fn registrations_outside_the_group_are_left_alone() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "local g = neotmux.group('t')
             g.on_tick(function() end)
             neotmux.on_tick(function() end)
             neotmux.group('t')",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert_eq!(ticks(&sub), 1);
}

/// Names are one namespace: two plugins that pick the same one share a group.
#[test]
fn two_sources_naming_the_same_group_share_it() {
    let (_dir, sub) = built(
        &[
            ("lua/plugin/a.lua", "neotmux.group('shared').on_tick(function() end)"),
            ("lua/plugin/b.lua", "neotmux.group('shared')"),
            ("init.lua", "require('plugin.a') require('plugin.b')"),
        ],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert_eq!(ticks(&sub), 0);
}

#[test]
fn a_group_call_answers_what_the_plain_call_does_refusals_included() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "local g = neotmux.group('t')
             local h = g.reserve('bottom', 1)
             assert(type(h) == 'table' and type(h.remove) == 'function')
             local none, why = g.reserve('bottom', 4)
             assert(none == nil and type(why) == 'string', 'the cap should refuse')
             local ok = pcall(g.reserve, 'nowhere', 1)
             assert(not ok, 'an unknown edge should raise through the group too')",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert!(logged(&sub).is_empty());
}

#[test]
fn a_name_that_is_not_a_non_empty_string_raises() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "assert(not pcall(neotmux.group, ''))
             assert(not pcall(neotmux.group, {}))",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert!(logged(&sub).is_empty());
}

/// The owner is still the chunk the call was written in, not the host's wrapper.
#[test]
fn a_group_call_is_attributed_to_the_chunk_that_made_it() {
    let (_dir, sub) = built(
        &[
            ("lua/plugin/banded.lua", "assert(neotmux.group('b').reserve('bottom', 1))"),
            ("init.lua", "require('plugin.banded')"),
        ],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    let owners: Vec<String> = sub
        .reservation_owners()
        .into_iter()
        .filter_map(|id| sub.sources().name(id))
        .collect();
    assert_eq!(owners, vec!["plugin/banded".to_string()]);
}

/// A job in an emptied group is killed, and its `on_exit` — the old code's — is never called.
#[test]
fn a_job_in_an_emptied_group_is_killed_without_its_on_exit() {
    let runner = support::FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"x\n".to_vec())]]);
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "neotmux.group('t').job.start({ 'sleep' }, {
               on_exit = function() neotmux.log.info('on_exit ran') end,
             })
             neotmux.group('t')",
        )],
        runner.clone(),
    );
    assert!(*runner.killed.borrow(), "the job ran on");
    sub.dispatch_jobs(support::one());
    assert!(!logged(&sub).iter().any(|l| l == "on_exit ran"));
}
```

`LuaSubsystem::census()` and `reservation_owners()` are added in this task (Step 3). `support::FakeRunner` is in `tests/support/mod.rs` already.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p neotmux-lua --test groups`
Expected: FAIL to compile (`census`, `reservation_owners` missing), and once those exist, `attempt to call a nil value (field 'group')`.

- [ ] **Step 3: Implement the types**

`neotmux-lua/src/plugin/group_id.rs`:

```rust
/// A group, as `neotmux.group` minted it. The name is the plugin's; the id is what the host
/// keys the group by, so a name is looked up once per call rather than once per registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroupId(pub u64);
```

`neotmux-lua/src/plugin/held.rs`:

```rust
use std::rc::Rc;

use mlua::RegistryKey;

use crate::draw::{FloatId, ReservationId};
use crate::plugin::RegistrationId;
use crate::sched::JobId;
use crate::sched::timers::TimerId;

/// One thing a group holds on the host, and what taking it back means.
pub enum Held {
    Registration(RegistrationId),
    Reservation(ReservationId),
    Float(FloatId),
    Job(JobId),
    Timer(TimerId),
    /// The callback as well as the name, so a command the source registered again since is
    /// not the one taken away.
    Command {
        name: String,
        callback: Rc<RegistryKey>,
    },
}
```

`neotmux-lua/src/plugin/in_group.rs`:

```rust
use crate::plugin::{GroupId, Groups};

/// The span of one `g.*` call. Whatever is registered while it lives is the group's. It puts
/// back what was there before rather than clearing, so a `g.*` reached from inside another is
/// still attributed to the inner group and the outer one resumes after.
pub struct InGroup<'a> {
    pub(crate) groups: &'a Groups,
    pub(crate) was: Option<GroupId>,
}

impl Drop for InGroup<'_> {
    fn drop(&mut self) {
        self.groups.current.set(self.was);
    }
}
```

`neotmux-lua/src/plugin/groups.rs`:

```rust
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use crate::plugin::{GroupId, Held, InGroup};

/// Every group there is, what each holds, and which one a `g.*` call is running for.
#[derive(Default)]
pub struct Groups {
    next: Cell<u64>,
    by_name: RefCell<BTreeMap<String, GroupId>>,
    held: RefCell<BTreeMap<GroupId, Vec<Held>>>,
    pub(crate) current: Cell<Option<GroupId>>,
    /// The groups named since `begin_reload`, while one is in progress.
    named: RefCell<Option<BTreeSet<GroupId>>>,
}

impl Groups {
    /// The group of that name, made if there is none, and what it held — which the caller
    /// takes back, since naming a group is emptying it.
    pub fn name(&self, name: &str) -> (GroupId, Vec<Held>) {
        let id = *self.by_name.borrow_mut().entry(name.to_string()).or_insert_with(|| {
            let id = GroupId(self.next.get() + 1);
            self.next.set(id.0);
            id
        });
        if let Some(named) = self.named.borrow_mut().as_mut() {
            named.insert(id);
        }
        let held = self.held.borrow_mut().remove(&id).unwrap_or_default();
        (id, held)
    }

    pub fn enter(&self, id: GroupId) -> InGroup<'_> {
        let was = self.current.replace(Some(id));
        InGroup { groups: self, was }
    }

    /// Files `held` under the group a `g.*` call is running for. Nothing outside one.
    pub fn note(&self, held: Held) {
        if let Some(id) = self.current.get() {
            self.held.borrow_mut().entry(id).or_default().push(held);
        }
    }

    pub fn begin_reload(&self) {
        *self.named.borrow_mut() = Some(BTreeSet::new());
    }

    /// Everything held by a group nobody named during the reload. Those groups are
    /// forgotten: the plugin that named them is not loaded any more.
    pub fn end_reload(&self) -> Vec<Held> {
        let named = self.named.borrow_mut().take().unwrap_or_default();
        let mut by_name = self.by_name.borrow_mut();
        let gone: Vec<GroupId> = by_name.values().copied().filter(|id| !named.contains(id)).collect();
        by_name.retain(|_, id| named.contains(id));
        let mut held = self.held.borrow_mut();
        gone.into_iter().flat_map(|id| held.remove(&id).unwrap_or_default()).collect()
    }
}
```

Add the four `mod x; pub use x::*;` pairs to `neotmux-lua/src/plugin/mod.rs`, in the file's existing order.

- [ ] **Step 4: `Jobs::remove`**

In `neotmux-lua/src/sched/jobs.rs`, beside `remove_all_of`:

```rust
    /// Kills and drops one job with no `Exit`, for the reason `remove_all_of` gives: its group
    /// is being emptied, and the callback that would hear the ending is the old code's.
    pub fn remove(&self, id: JobId) {
        for mut job in self.take_matching(|job| job.id == id) {
            let _ = job.running.kill();
        }
    }
```

- [ ] **Step 5: Move `Sweep` and teach it `empty`**

Create `neotmux-lua/src/api/sweep.rs` with the `Sweep` struct and `remove_all_of` moved verbatim from `api/install.rs` (fields become `pub(crate)`, derive `Clone`, add `commands: Rc<Commands>`), plus:

```rust
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
```

(`SourceId` must be `Ord` for `sort`; if it is not, collect into a `BTreeSet` only if it is, otherwise `dedup_by` after pushing in order — check `plugin/source_id.rs`'s derives and use what it has.) Add `mod sweep; pub(crate) use sweep::*;` to `api/mod.rs`; in `install.rs` import it and delete the moved code.

- [ ] **Step 6: Note what each call adds**

Add `pub(crate) groups: Rc<Groups>` to `Shared` and thread `Rc<Groups>` to every add site:

- `install.rs` `register(...)`: after `registry.add`, `groups.note(Held::Registration(id));`
- `keymap.rs` `install` gains `groups: Rc<Groups>`; after `registry.add`, `groups.note(Held::Registration(id));`
- `reserve.rs` `install` gains `groups`; in the `Ok(id)` arm, `groups.note(Held::Reservation(id));`
- `float.rs` `install` gains `groups`; after `floats.add`, `groups.note(Held::Float(id));`
- `job.rs` `install` gains `groups`; in `Ok(id)`, `groups.note(Held::Job(id));`
- `timer.rs` `install` gains `groups`; in `Ok(id)`, `groups.note(Held::Timer(id));`
- `command.rs` `install` gains `groups`; the `key` is cloned before `commands.add(...)`, and in `Ok(())`, `groups.note(Held::Command { name: name.clone(), callback: key_clone });`

In `LuaSubsystem::build_with_runner`, make `let groups = Rc::new(Groups::default());`, pass it in `Shared`, and keep it as a field `groups: Rc<Groups>`.

- [ ] **Step 7: `neotmux.group`**

Create `neotmux-lua/src/api/group.rs`:

```rust
//! `neotmux.group(name)`: nvim's augroup with `clear = true`, over every registration.

use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table};

use crate::api::Sweep;
use crate::plugin::{GroupId, Groups};

const TOP: [&str; 6] = ["on_tick", "on_event", "on_key", "on_write", "float", "reserve"];
const NESTED: [(&str, &[&str]); 4] = [
    ("keymap", &["set"]),
    ("command", &["add"]),
    ("job", &["start"]),
    ("timer", &["every", "after"]),
];

fn wrap(lua: &Lua, groups: &Rc<Groups>, id: GroupId, real: Function) -> mlua::Result<Function> {
    let groups = Rc::clone(groups);
    lua.create_function(move |_, args: MultiValue| {
        let _in = groups.enter(id);
        real.call::<MultiValue>(args)
    })
}

/// `neotmux` is the real table, before it is sealed: the wrappers call what is in it.
pub(crate) fn install(
    lua: &Lua,
    neotmux: &Table,
    groups: Rc<Groups>,
    sweep: Sweep,
    seal: fn(&Lua, Table, &'static str) -> mlua::Result<Table>,
) -> mlua::Result<Function> {
    let neotmux = neotmux.clone();
    lua.create_function(move |lua, name: mlua::Value| {
        let name = match name {
            mlua::Value::String(s) if !s.as_bytes().is_empty() => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::RuntimeError(
                    "neotmux.group takes a name, a non-empty string".to_string(),
                ));
            }
        };
        let (id, held) = groups.name(&name);
        sweep.empty(held);
        let g = lua.create_table()?;
        for field in TOP {
            g.set(field, wrap(lua, &groups, id, neotmux.get(field)?)?)?;
        }
        for (table, fields) in NESTED {
            let real: Table = neotmux.get(table)?;
            let sub = lua.create_table()?;
            for field in fields {
                sub.set(*field, wrap(lua, &groups, id, real.get(*field)?)?)?;
            }
            g.set(table, seal(lua, sub, "a group's table")?)?;
        }
        seal(lua, g, "a group")
    })
}
```

In `install.rs`, make `seal` `pub(crate)`, and just before `let neotmux = seal(lua, neotmux, "neotmux")?;`:

```rust
    {
        let group = super::group::install(lua, &neotmux, groups.clone(), sweep.clone(), seal)?;
        neotmux.set("group", group)?;
    }
```

where `sweep` is the `Sweep` value now built before this point (build it once, clone it into the require wrapper as well). Add `mod group;` to `api/mod.rs`.

- [ ] **Step 8: The two test accessors**

In `neotmux-lua/src/lua_subsystem.rs`:

```rust
    /// What the host is holding for the Lua layer, counted — what a reload is checked against.
    pub fn census(&self) -> Census {
        Census {
            registrations: self.registry.kinds(),
            commands: self.commands.list().into_iter().map(|c| c.name).collect(),
            reserved: self.reservations.totals(),
            floats: self.floats.count(),
            jobs: self.jobs.len(),
            timers: self.timers.len(),
        }
    }

    /// Who holds each band, in reservation order.
    pub fn reservation_owners(&self) -> Vec<SourceId> {
        self.reservations.owners()
    }
```

Create `neotmux-lua/src/census.rs` (flattened from `lib.rs`):

```rust
use neotmux_proto::Reserved;

/// A count of everything the host holds for the Lua layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    /// Every registration's kind, `Debug`-printed and sorted: `Tick`, `Event("resized")`, …
    pub registrations: Vec<String>,
    pub commands: Vec<String>,
    pub reserved: Reserved,
    pub floats: usize,
    pub jobs: usize,
    pub timers: usize,
}
```

and add `Registry::kinds(&self) -> Vec<String>` (each entry's `format!("{:?}", kind)`, sorted), `Jobs::len`, `Timers::len` (count of `armed` whose `spent` is false), `Reservations::owners` (owners in reservation order). Check `Reserved` derives `PartialEq, Eq, Default, Debug` in `neotmux-proto`; it is compared in existing tests.

- [ ] **Step 9: Run the tests**

Run: `cargo test -p neotmux-lua --test groups`
Expected: PASS. Then `cargo test --workspace` — PASS (the signature changes touch every binding).

- [ ] **Step 10: Commit**

```bash
git add -A && git commit -m "neotmux.group: a named set of registrations that empties itself when it is named again"
```

---

### Task 3: `unload`, and remembering which modules the layers loaded

**Files:**
- Modify: `neotmux-lua/src/plugin/sources.rs` (`note_module`, `modules`; the searcher notes)
- Modify: `neotmux-lua/src/lua_subsystem.rs` (`dispatch_unload`)
- Test: `neotmux-lua/tests/events.rs`

**Interfaces:**
- Produces: `Sources::modules(&self) -> Vec<String>` (every module name the layer searcher resolved, in load order, without duplicates); `LuaSubsystem::dispatch_unload(&self, session: Option<SessionId>)`.

- [ ] **Step 1: Failing test**

In `neotmux-lua/tests/events.rs` (use its existing builder):

```rust
/// `unload` has no fields and reaches every subscriber once.
#[test]
fn unload_reaches_every_subscriber_with_an_empty_table() {
    let (_dir, sub) = built(
        "neotmux.on_event('unload', function(e) neotmux.log.info('a ' .. tostring(next(e))) end)
         neotmux.on_event('unload', function() neotmux.log.info('b') end)",
    );
    sub.dispatch_unload(Some(support::one()));
    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "a nil"), "{said:?}");
    assert!(said.iter().any(|l| l == "b"), "{said:?}");
}
```

- [ ] **Step 2: Run** — `cargo test -p neotmux-lua --test events unload` → FAIL (no `dispatch_unload`).

- [ ] **Step 3: Implement**

`LuaSubsystem`:

```rust
    /// The one event the host raises for itself rather than for a pane: the config is about to
    /// run again. Bound to the session that asked, if it is still there.
    pub fn dispatch_unload(&self, session: Option<SessionId>) {
        let _bound = session.map(|session| self.ambient.bind(session));
        let subscribers: Vec<_> = self
            .registry
            .snapshot(KindTag::Event)
            .into_iter()
            .filter(|id| matches!(self.registry.kind(*id), Some(Kind::Event(name)) if name == "unload"))
            .collect();
        if subscribers.is_empty() {
            return;
        }
        self.measured(|| {
            for id in subscribers {
                let (Some(callback), Some(owner)) = (self.registry.callback(id), self.registry.owner(id)) else {
                    continue;
                };
                let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                    continue;
                };
                let called = self.lua.create_table().and_then(|fields| func.call::<()>(fields));
                if let Err(err) = called {
                    self.report(owner, &err);
                }
            }
        });
    }
```

`Sources`: add `modules: RefCell<Vec<String>>`, and

```rust
    pub fn note_module(&self, name: &str) {
        let mut modules = self.modules.borrow_mut();
        if !modules.iter().any(|known| known == name) {
            modules.push(name.to_string());
        }
    }

    pub fn modules(&self) -> Vec<String> {
        self.modules.borrow().clone()
    }
```

In `install_loader`'s searcher, after `sources.intern(&chunk_name);`, `sources.note_module(&modname);`.

- [ ] **Step 4: Run** — PASS.

- [ ] **Step 5: Docs** — `docs/plugins.md` event table: add a row `| unload | (none) | the config is about to be reloaded; see Reloading |`. The "There are no others." sentence under the table stays; it is still true with the row added.

- [ ] **Step 6: Commit** — `git commit -am "unload: the event a plugin hears before the config runs again"`

---

### Task 4: `LuaSubsystem::reload` and `neotmux.reload()`

**Files:**
- Modify: `neotmux-lua/src/host/op.rs` (`Op::Reload`)
- Modify: `neotmux-lua/src/api/install.rs` (`neotmux.reload`)
- Modify: `neotmux-lua/src/lua_subsystem.rs` (`reload`)
- Test: `neotmux-lua/tests/reload.rs` (new)

**Interfaces:**
- Consumes: `Groups::begin_reload/end_reload`, `Sweep::empty` (Task 2); `dispatch_unload`, `Sources::modules` (Task 3).
- Produces: `LuaSubsystem::reload(&self, session: Option<SessionId>) -> Vec<LoadFailure>`; `Op::Reload`; Lua `neotmux.reload()`.

- [ ] **Step 1: Failing tests**

Create `neotmux-lua/tests/reload.rs`:

```rust
//! A reload: `unload`, forget the layers' modules, run every `init.lua` again, sweep the
//! groups nobody named.

use std::fs;
use std::path::Path;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::Op;
use neotmux_lua::plugin::{KindTag, Layers};

mod support;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        write(dir.path(), rel, body);
    }
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()]), support::panes()).unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(None)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

const CLOCK: &str = "local g = neotmux.group('plugin.clock')
                     g.on_tick(function() end)
                     g.on_event('unload', function() neotmux.log.info('clock unloading') end)";

#[test]
fn a_reload_runs_the_new_code_and_leaves_none_of_the_old() {
    let (dir, sub) = built(&[("lua/plugin/clock.lua", CLOCK), ("init.lua", "require('plugin.clock')")]);
    write(
        dir.path(),
        "lua/plugin/clock.lua",
        "local g = neotmux.group('plugin.clock')
         g.on_tick(function() end)
         g.on_tick(function() end)",
    );
    assert!(sub.reload(None).is_empty());
    assert_eq!(sub.registry().snapshot(KindTag::Tick).len(), 2);
}

#[test]
fn unload_runs_once_before_the_reload() {
    let (_dir, sub) = built(&[("lua/plugin/clock.lua", CLOCK), ("init.lua", "require('plugin.clock')")]);
    logged(&sub);
    sub.reload(None);
    let said = logged(&sub);
    assert_eq!(said.iter().filter(|l| *l == "clock unloading").count(), 1, "{said:?}");
}

#[test]
fn a_plugin_whose_require_was_deleted_is_gone_after_a_reload() {
    let (dir, sub) = built(&[("lua/plugin/clock.lua", CLOCK), ("init.lua", "require('plugin.clock')")]);
    write(dir.path(), "init.lua", "");
    sub.reload(None);
    assert!(sub.registry().snapshot(KindTag::Tick).is_empty());
}

/// A plugin that now fails is swept and reported, and the rest of the config still loads.
#[test]
fn a_plugin_that_now_fails_does_not_stop_the_rest_loading() {
    let (dir, sub) = built(&[
        ("lua/plugin/clock.lua", CLOCK),
        ("lua/plugin/other.lua", "neotmux.group('plugin.other').on_tick(function() end)"),
        ("init.lua", "require('plugin.clock') require('plugin.other')"),
    ]);
    write(dir.path(), "lua/plugin/clock.lua", "local g = neotmux.group('plugin.clock') g.on_tick(function() end) error('broken')");
    let failures = sub.reload(None);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(sub.registry().snapshot(KindTag::Tick).len(), 1, "only plugin.other's tick");
}

/// A group named lazily, from a callback, is swept by a reload that does not name it.
#[test]
fn a_group_named_from_a_callback_is_swept_by_a_reload_that_does_not_name_it() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.keymap.set('C-x', function() neotmux.group('lazy').on_tick(function() end) return true end)",
    )]);
    sub.dispatch_key(support::one(), "C-x");
    assert_eq!(sub.registry().snapshot(KindTag::Tick).len(), 1);
    assert!(sub.reload(None).is_empty());
    assert!(sub.registry().snapshot(KindTag::Tick).is_empty());
}

#[test]
fn neotmux_reload_raises_the_op_and_does_not_reload_where_it_runs() {
    let (_dir, sub) = built(&[("init.lua", "neotmux.keymap.set('C-r', function() neotmux.reload() return true end)")]);
    sub.dispatch_key(support::one(), "C-r");
    let ops = sub.drain_ops(Some(support::one()));
    assert!(ops.iter().any(|(_, op)| *op == Op::Reload), "{ops:?}");
}
```

- [ ] **Step 2: Run** — `cargo test -p neotmux-lua --test reload` → FAIL (`reload` missing).

- [ ] **Step 3: Implement**

`Op`: add a variant

```rust
    /// Run the config again: the host does it after the frame, never inside the callback that
    /// asked, since that callback is running in the VM being reloaded.
    Reload,
```

and handle the new variant wherever `Op` is matched exhaustively in `neotmux-lua` (the compiler lists them).

`install.rs`, beside `quit`:

```rust
    {
        let sources = sources.clone();
        let ops = ops.clone();
        let reload = lua.create_function(move |lua, ()| {
            ops.push(plugin::caller(lua, &sources), Op::Reload);
            Ok(())
        })?;
        neotmux.set("reload", reload)?;
    }
```

`LuaSubsystem`:

```rust
    /// The config, run again in this VM: `unload`, every module the layers loaded forgotten,
    /// every layer's `init.lua` run as at start, and every group nobody named taken back.
    /// Never called from inside Lua — the host calls it between frames.
    pub fn reload(&self, session: Option<SessionId>) -> Vec<LoadFailure> {
        self.dispatch_unload(session);
        self.groups.begin_reload();
        match self.lua.globals().get::<Table>("package").and_then(|p| p.get::<Table>("loaded")) {
            Ok(loaded) => {
                for name in self.sources.modules() {
                    if let Err(err) = loaded.set(name, mlua::Value::Nil) {
                        self.report(crate::plugin::ANONYMOUS, &err);
                    }
                }
            }
            Err(err) => self.report(crate::plugin::ANONYMOUS, &err),
        }
        let failures = self.run_init();
        self.sweep().empty(self.groups.end_reload());
        failures
    }

    fn sweep(&self) -> crate::api::Sweep {
        crate::api::Sweep {
            registry: self.registry.clone(),
            buffers: self.buffers.clone(),
            reservations: self.reservations.clone(),
            floats: self.floats.clone(),
            jobs: self.jobs.clone(),
            timers: self.timers.clone(),
            commands: self.commands.clone(),
        }
    }
```

- [ ] **Step 4: Run** — `cargo test -p neotmux-lua --test reload` → PASS; `cargo test --workspace` → PASS.

- [ ] **Step 5: Commit** — `git commit -am "reload: the config runs again in the same VM, and every group nobody named goes"`

---

### Task 5: The server carries a reload out after the frame

**Files:**
- Modify: `neotmux-server/src/server_op.rs` (`ServerOp::Reload`)
- Modify: `neotmux-server/src/plugins.rs` (`Op::Reload` → `ServerOp::Reload`; `drain_load_ops` refuses it; `Plugins::reload`)
- Modify: `neotmux-server/src/server.rs` (`apply_server_ops`: reload once, last)
- Test: `neotmux-server/src/plugins.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `LuaSubsystem::reload` (Task 4).
- Produces: `Plugins::reload(&mut self, session: Option<SessionId>)` — runs the reload, logs every failure at warn, then drains the ops the reload raised: logs and notifies are kept; every other op is refused with `refuse(&tag, "<op>", "the config was reloading, and a load acts on no session")`.

- [ ] **Step 1: Failing test** (in `plugins.rs`'s test module, using its `built` / `begin`):

```rust
    #[test]
    fn a_reload_raised_by_a_key_reaches_the_server_as_a_server_op() {
        let (_dir, mut plugins) = built(
            "neotmux.keymap.set('C-r', function() neotmux.reload() return true end)",
        );
        let session = SessionId::from(1);
        begin(&plugins, session);
        plugins.key(session, Key::from_spec("C-r").unwrap());
        let (_ops, server) = plugins.drain_ops(Some(session), &BTreeMap::new());
        assert!(server.iter().any(|(_, op)| matches!(op, ServerOp::Reload)), "{server:?}");
    }
```

(`ServerOp` needs `Debug`; derive it if it lacks it.)

- [ ] **Step 2: Run** — `cargo test -p neotmux-server a_reload_raised` → FAIL.

- [ ] **Step 3: Implement**

`ServerOp`: add `Reload,`. `drain_ops`: `Op::Reload => { server.push((tag, ServerOp::Reload)); continue; }`. `drain_load_ops`: `ServerOp::Reload => ("neotmux.reload()".to_string(), "the config is loading already")`. `drain_teardown_ops` drops it with the rest.

`Plugins::reload`:

```rust
    pub fn reload(&mut self, session: Option<SessionId>) {
        for failure in self.lua.reload(session) {
            logging::log(Level::Warn, &failure.source, &failure.message);
        }
        let (ops, server) = self.drain_ops(None, &BTreeMap::new());
        let why = "the config was reloading, and a load acts on no session";
        for (tag, op) in server {
            refuse(&tag, &format!("{op:?}"), why);
        }
        for (tag, op) in ops {
            refuse(&tag, &format!("{op:?}"), why);
        }
    }
```

`server.rs`, `apply_server_ops`: before the `for`, `let mut reload: Option<SessionId> = None;`; add the arm `ServerOp::Reload => { reload.get_or_insert(from); }`; after the loop:

```rust
    // Last, and once however many asked: every other op this frame was raised by the old
    // code and is carried out as the old code meant it.
    if let Some(from) = reload {
        plugins.reload(sessions.get_mut(from).is_some().then_some(from));
    }
```

- [ ] **Step 4: Run** — PASS; `cargo test --workspace` → PASS.

- [ ] **Step 5: Commit** — `git commit -am "the server reloads the config after the frame that asked, once"`

---

### Task 6: The shipped runtime and examples use groups; `config-reload`

**Files:**
- Modify: every file listed by `grep -rlE "neotmux\.(on_tick|on_event|on_key|on_write|reserve|float|keymap\.set|command\.add|job\.start|timer\.)" runtime/ examples/` — at the time of writing: `runtime/lua/neotmux/{detect,guard,hints,list,mode,prompt,seen,state}.lua`, `runtime/lua/plugin/{borders,chime,copy,footer,guard,hints,notify,palette,prefix,sessions,sidebar,status,sync,watch}.lua`, `runtime/lua/plugin/sidebar/{agents,spaces}.lua`, `examples/{clock,pane_labels,prefix}.lua`
- Modify: `runtime/init.lua` (register `config-reload`)
- Test: `neotmux-lua/tests/reload.rs` (shipped-runtime tests)

- [ ] **Step 1: Failing tests** — append to `neotmux-lua/tests/reload.rs`:

```rust
fn shipped_with_everything_on() -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "init.lua",
        "local d = require('neotmux.defaults')
         for k in pairs(d) do d[k] = true end",
    );
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), support::shipped_layer()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

/// The test the design rests on: a reload leaves what a start leaves.
#[test]
fn a_reload_of_the_shipped_runtime_leaves_what_a_start_leaves() {
    let (_dir, sub) = shipped_with_everything_on();
    let started = sub.census();
    assert!(sub.reload(None).is_empty());
    assert_eq!(sub.census(), started);
    assert!(sub.reload(None).is_empty());
    assert_eq!(sub.census(), started, "a second reload drifted");
}

#[test]
fn config_reload_is_a_command() {
    let (_dir, sub) = shipped_with_everything_on();
    assert!(sub.commands().iter().any(|c| c.name == "config-reload"));
}
```

Add to `neotmux-lua/tests/copy.rs` (its `built`, `enter` and `pane` helpers; `Built::terminals()` answers `is_frozen` straight from the panes):

```rust
/// A reload with copy mode open leaves the pane moving again: the new copy mode never
/// entered anything, so only the old one's `unload` can thaw what it froze.
#[test]
fn a_reload_with_copy_mode_open_leaves_no_pane_frozen() {
    let (_dir, sub) = built("");
    enter(&sub);
    assert_eq!(sub.terminals().is_frozen(pane()), Some(true), "copy mode did not freeze the pane");
    sub.reload(Some(support::one()));
    assert_eq!(sub.terminals().is_frozen(pane()), Some(false), "the reload left the pane frozen");
}
```

Add to `neotmux-lua/tests/hints.rs` (its `built`, `default_trigger`, `feed` and `open` helpers):

```rust
#[test]
fn a_reload_with_a_pick_open_leaves_no_pane_frozen() {
    let (_dir, sub) = built(&default_trigger());
    feed(&sub, b"http://example.com");
    open(&sub);
    let one = neotmux_proto::PaneId::from(1);
    assert_eq!(sub.terminals().is_frozen(one), Some(true), "hints did not freeze the pane");
    sub.reload(Some(support::one()));
    assert_eq!(sub.terminals().is_frozen(one), Some(false), "the reload left the pane frozen");
}
```

- [ ] **Step 2: Run** — `cargo test -p neotmux-lua --test reload --test copy --test hints` → the census test FAILS (ticks and events double), `config_reload_is_a_command` FAILS, the freeze tests FAIL.

- [ ] **Step 3: Convert each file**

The rule, for every file in the list: directly after the file's `require` block, add

```lua
local g = neotmux.group("<the module's name, as `require` names it>")
```

and replace every `neotmux.on_tick`, `neotmux.on_event`, `neotmux.on_key`, `neotmux.on_write`, `neotmux.reserve`, `neotmux.float`, `neotmux.keymap.set`, `neotmux.command.add`, `neotmux.job.start`, `neotmux.timer.every` and `neotmux.timer.after` call with `g.` in place of `neotmux.`. For example `runtime/lua/plugin/sync.lua`:

```lua
local g = neotmux.group("plugin.sync")
-- …
g.command.add("sync-panes", function()
  return M.toggle()
end, { desc = "type into every pane of this window at once" })
```

A file that registers lazily (from inside a function, e.g. `neotmux.list`'s `on_key`/`on_tick` armed on first open, `neotmux.hints`'s `registered` flag) still names its group at the top of the file, so a reload empties it; the lazy calls use the same `g`. Nothing else in these files changes. `prefix.lua`'s `register` helper calls `neotmux.command.add` — it becomes `g.command.add`.

Then the two `unload` hooks. `runtime/lua/plugin/copy.lua`, after its registrations:

```lua
-- A reload runs this file again with a copy mode that has not entered anything; the pane
-- this one froze would stay frozen with nobody left who knows it.
g.on_event("unload", function()
  leave()
end)
```

using the name of the function in `copy.lua` that ends the mode and thaws (the block at lines 80–95 is its body; call that function). `runtime/lua/neotmux/hints.lua`:

```lua
g.on_event("unload", close)
```

(`close` at line 170 thaws every pane in `state.frozen`.)

`runtime/init.lua`, at the end:

```lua
-- Here rather than in a plugin: turning every shipped plugin off must not take the way back.
neotmux.group("neotmux.reload").command.add("config-reload", function()
  neotmux.reload()
end, { desc = "run the config again, keeping the panes" })
```

- [ ] **Step 4: Run** — `cargo test -p neotmux-lua` → PASS; `make lua lint` → clean; `cargo test --workspace` → PASS.

- [ ] **Step 5: Commit** — `git commit -am "the shipped runtime and the examples register through groups, copy and hints thaw on unload, and config-reload is a command"`

---

### Task 7: Docs, CHANGELOG, and a real session

**Files:**
- Modify: `docs/plugins.md` ("Registering" table, "Reloading", lines ~346 and ~1103)
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Docs**

- "Registering": add the row `| neotmux.group(name) | nothing runs; answers a table of the calls above that files what they register under name, emptied first (Reloading) |`.
- Replace the whole "Reloading" section with: what `config-reload` / `neotmux.reload()` does (the six steps of spec §3 in plain prose); the group idiom with a five-line example (`local g = neotmux.group('plugin.clock')`, `g.on_tick`, `g.reserve`); that a registration made with `neotmux.*` outside a group is there twice after a reload; that `unload` is for what no group can see, with `pane.freeze` as the example; that a plugin's module locals start again.
- Find the two sentences saying a re-attach re-runs `init.lua` (`grep -n "re-attach\|Re-attaching" docs/plugins.md`) and rewrite them: `init.lua` runs once, when the server starts, and again on `config-reload`; a re-attach runs nothing.

- [ ] **Step 2: CHANGELOG** — under `## Unreleased`, a section `### The config reloads without losing the panes` in the file's voice: `config-reload` (palette, CLI), `neotmux.reload()`, `neotmux.group`, `unload`, same-source keys and commands replace, `command.remove`.

- [ ] **Step 3: A real session**

Build and install to a scratch prefix, start a session, and check by hand:
1. `C-b :` → `config-reload` → the status bar, borders and sidebar redraw once; nothing is doubled (open the palette: each command appears once).
2. Enter copy mode (`C-b [`), run `config-reload` from the CLI (`neotmux` command form — see `docs/` for the exact spelling), and check the pane moves again and copy mode is gone.
3. Edit `~/.config/neotmux/init.lua` to set `require('neotmux.defaults').sidebar = true`, reload, and see the sidebar appear; set it back, reload, and see it go.
4. Type a key straight after choosing `config-reload` in the palette: it reaches the pane (new code) and nothing is lost.

Record what was seen in the commit message body if anything differed from expected, and fix it before committing.

- [ ] **Step 4: Full check** — `make check` → clean.

- [ ] **Step 5: Commit** — `git commit -am "docs: the config reloads, how a plugin is written for it, and a re-attach runs nothing"`

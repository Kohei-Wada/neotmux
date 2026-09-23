# A write to a pane passes a hook — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `neotmux.on_write(fn)` — every write of text into a pane is offered to Lua first, and
a handler that refuses it, raises, or overruns stops it.

**Architecture:** One more subscription kind in the Lua registry (`Kind::Write`), dispatched by
the host at the two places text reaches a pane: the `send` command, answered at the door where
the VM is idle, and `Op::PaneSend`, converted after the tick that raised it. With no handler
registered nothing changes — the registry answers "is anyone listening" without entering Lua.

**Tech Stack:** Rust 2024, one workspace, no new dependencies. mlua for the callbacks,
`portable-pty` for the shells that already exist.

**Spec:** `docs/superpowers/specs/2026-09-24-writes-pass-a-hook-design.md`

## Global Constraints

- **Failure is a refusal.** A handler that returns `false`, raises, overruns its budget, or
  cannot be reached refuses the write. Nothing in this plan may make a failure fall through to
  the write.
- **With no handler registered, the behaviour is exactly today's**: `send` is answered at the
  door, in Rust, and the Lua layer is not entered.
- The first refusal is the answer; handlers after it are not called.
- A handler **cannot rewrite** the text. The only answers are pass and refuse.
- A refusal is told three ways: the caller (non-zero exit and the sentence on stderr, or
  `false, why` for `pane.send`), the person (a notification), and the log (`warn`).
- The human's keystrokes are **not** writes and are not hooked.
- Every task ends green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test --workspace`. Two PTY tests
  (`socket::ending_one_session_leaves_the_other_typing_into_its_own_shell`,
  `terminal_restore::renders_output_written_in_the_same_batch_as_exit`) flake under parallel
  load — re-run alone before blaming the branch.
- Commit messages are prose in this repo's voice: a lower-case summary line that says what is
  true now, then why. No `feat:` prefixes, no Co-Authored-By trailers.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/neotmux-lua/src/registry.rs` | `Kind::Write` / `KindTag::Write` |
| `crates/neotmux-lua/src/api/mod.rs` | `neotmux.on_write` registration |
| `crates/neotmux-lua/src/lib.rs` | `dispatch_write`, `has_write_hooks`, `WriteVerdict` |
| `crates/neotmux/src/plugins.rs` | the host's one entry point: `check_write`, and the `Op::PaneSend` path |
| `crates/neotmux/src/command.rs` | `send` asking before it writes |
| `runtime/lua/neotmux/guard.lua` | the convention: pattern tables on top of the hook |
| `docs/plugins.md` | the hook, what it covers, and what it does not |

---

### Task 1: The hook exists and answers

**Files:**
- Modify: `crates/neotmux-lua/src/registry.rs`
- Modify: `crates/neotmux-lua/src/api/mod.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`
- Test: `crates/neotmux-lua/tests/writes.rs` (create), `crates/neotmux-lua/tests/subsystem.rs`

**Interfaces:**
- Produces:
  ```rust
  /// What the handlers said about a write. `Refused` carries what the caller is told.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum WriteVerdict {
      Pass,
      Refused(String),
  }

  impl LuaSubsystem {
      /// Whether anything is listening, answered without entering Lua.
      pub fn has_write_hooks(&self) -> bool;
      /// Offers a write to every handler, in registration order, stopping at the first
      /// refusal. A handler that raises or overruns refuses it.
      pub fn dispatch_write(
          &self,
          session: SessionId,
          pane: PaneId,
          text: &str,
          from: &str,
      ) -> WriteVerdict;
  }
  ```

- [ ] **Step 1: Write the failing tests**

Create `crates/neotmux-lua/tests/writes.rs`. Follow `tests/panes.rs` for the harness — `built`,
`support::one()`, `side_by_side` — and copy what it does rather than importing from it.

```rust
/// The handler sees the write exactly as it was asked for, and passing it is the default:
/// a handler that answers nothing has no opinion.
#[test]
fn a_handler_that_says_nothing_passes_the_write() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_write(function(write)
           neotmux.log.info(write.pane .. ' ' .. write.text .. ' ' .. write.from)
         end)",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "ls -l\n", "tool"),
        WriteVerdict::Pass
    );
    assert_eq!(logged(&sub, Some(support::one())), vec!["1 ls -l\n tool".to_string()]);
}

#[test]
fn a_handler_that_refuses_stops_the_write_and_its_reason_is_the_answer() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_write(function() return false, 'not on my watch' end)",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "rm -rf /", "tool"),
        WriteVerdict::Refused("not on my watch".to_string())
    );
}

/// A refusal is a decision, not a vote: nothing after it runs.
#[test]
fn the_handlers_after_a_refusal_are_not_called() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", "neotmux.on_write(function() return false, 'first' end) return {}"),
        (
            "lua/plugin/b.lua",
            "neotmux.on_write(function() neotmux.log.info('b ran') end) return {}",
        ),
    ]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "x", "tool"),
        WriteVerdict::Refused("first".to_string())
    );
    assert!(logged(&sub, Some(support::one())).is_empty(), "a handler past the refusal ran");
}

/// The rule this whole thing rests on: a guardrail that fails open is useless exactly when
/// something is wrong.
#[test]
fn a_handler_that_raises_refuses_the_write() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_write(function() error('the guard itself is broken') end)",
    )]);
    side_by_side(&sub);
    let WriteVerdict::Refused(why) = sub.dispatch_write(support::one(), PaneId::from(1), "x", "tool")
    else {
        panic!("a raising handler passed the write");
    };
    assert!(why.contains("init"), "{why}");
}

#[test]
fn a_handler_that_overruns_its_budget_refuses_the_write() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.opt.tick_budget_ms = 5
         neotmux.on_write(function() while true do end end)",
    )]);
    side_by_side(&sub);
    assert!(matches!(
        sub.dispatch_write(support::one(), PaneId::from(1), "x", "tool"),
        WriteVerdict::Refused(_)
    ));
}

/// The cheap question the door asks before it decides which path a `send` takes.
#[test]
fn nothing_registered_is_nothing_listening() {
    let (_dir, sub) = built(&[("init.lua", "neotmux.on_tick(function() end)")]);
    assert!(!sub.has_write_hooks());
    let (_dir, sub) = built(&[("init.lua", "neotmux.on_write(function() end)")]);
    assert!(sub.has_write_hooks());
}
```

`neotmux.opt.tick_budget_ms` is the spelling `tests/budget.rs` uses — check it there and copy
it exactly.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test writes 2>&1 | tail -20`
Expected: FAIL — `on_write` is not a field of the sealed table, `dispatch_write` does not exist.

- [ ] **Step 3: Add the kind**

In `crates/neotmux-lua/src/registry.rs`, add `Write` to both `Kind` and `KindTag` and to
`Kind::tag`'s match — three one-line edits, matching `Key`.

- [ ] **Step 4: Install the registration**

In `crates/neotmux-lua/src/api/mod.rs`, beside the `on_key` block:

```rust
    {
        let sources = sources.clone();
        let registry = registry.clone();
        let buffers = buffers.clone();
        // Every write of text into a pane, before it is written. A handler answers nothing to
        // pass it and `false, why` to refuse it; `on_key` is the other half of this, for the
        // keys a person types, which are not writes.
        let on_write = lua.create_function(move |lua, callback: Function| {
            Ok(register(lua, &sources, &registry, &buffers, Kind::Write, callback)?.1)
        })?;
        neotmux.set("on_write", on_write)?;
    }
```

- [ ] **Step 5: Dispatch it**

In `crates/neotmux-lua/src/lib.rs`, beside `dispatch_key`. Note the budget: `measured` swallows
the outcome, and this needs it, so this arms and leaves the budget itself the way `measured`
does and turns a non-`WithinBudget` outcome into a refusal.

```rust
/// What the handlers said about a write. `Refused` carries the sentence the caller is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteVerdict {
    Pass,
    Refused(String),
}

impl LuaSubsystem {
    /// Whether anything is listening. Answered off the registry, so the door can ask it
    /// without entering Lua — which is what keeps a server with no handler on the path it
    /// has today.
    pub fn has_write_hooks(&self) -> bool {
        !self.registry.snapshot(KindTag::Write).is_empty()
    }

    /// Offers a write to every handler in registration order, stopping at the first refusal.
    ///
    /// Everything that is not a pass is a refusal: a handler that returns `false`, one that
    /// raises, one that overruns its budget, and a table this cannot be built in. A
    /// guardrail that fails open is useless exactly when something is wrong.
    pub fn dispatch_write(
        &self,
        session: SessionId,
        pane: PaneId,
        text: &str,
        from: &str,
    ) -> WriteVerdict {
        let _bound = self.ambient.bind(session);
        if let Err(err) = Budget::arm(&self.lua, &self.budget) {
            self.report_text(source::ANONYMOUS, format!("the budget is off: {err}"));
        }
        let budget_ms = self.opts.borrow().tick_budget_ms;
        self.budget.enter(Duration::from_millis(budget_ms));
        let verdict = self.offer_write(pane, text, from);
        let outcome = self.budget.leave();
        self.budget.disarm(&self.lua);
        if outcome != Outcome::WithinBudget {
            return WriteVerdict::Refused(format!(
                "a write handler exceeded the {budget_ms} ms budget"
            ));
        }
        verdict
    }

    fn offer_write(&self, pane: PaneId, text: &str, from: &str) -> WriteVerdict {
        for id in self.registry.snapshot(KindTag::Write) {
            let Some(callback) = self.registry.callback(id) else {
                continue;
            };
            let Some(owner) = self.registry.owner(id) else {
                continue;
            };
            let tag = self.sources.name(owner).unwrap_or_default();
            let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                return WriteVerdict::Refused(format!("{tag}: the handler could not be read"));
            };
            let write = match self.lua.create_table() {
                Ok(write) => write,
                Err(err) => {
                    return WriteVerdict::Refused(format!("{tag}: {err}"));
                }
            };
            if write.set("pane", pane.get()).is_err()
                || write.set("text", text).is_err()
                || write.set("from", from).is_err()
            {
                return WriteVerdict::Refused(format!("{tag}: the write could not be described"));
            }
            match func.call::<(Value, Value)>(write) {
                Ok((Value::Boolean(false), why)) => {
                    let why = match why {
                        Value::String(why) => why.to_string_lossy().to_string(),
                        _ => format!("refused by {tag}"),
                    };
                    return WriteVerdict::Refused(why);
                }
                Ok(_) => continue,
                Err(err) => {
                    self.report(owner, &err);
                    return WriteVerdict::Refused(format!("{tag}: {err}"));
                }
            }
        }
        WriteVerdict::Pass
    }
}
```

Export `WriteVerdict` from the crate root the way `KeyOutcome` is exported.

- [ ] **Step 6: Update the ABI fixture**

`crates/neotmux-lua/tests/subsystem.rs` has a test that lists every field of the sealed
top-level table (`pairs_over_the_sealed_top_level_table_sees_the_real_fields`). Add `on_write`
to its expected string, in the sorted position.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p neotmux-lua`
Expected: PASS, including the new file.

- [ ] **Step 8: Commit**

```bash
git add crates/neotmux-lua
git commit -m "$(cat <<'EOF'
a write to a pane can be offered to lua

`on_write` is the subscription; the host asking it is the next task. What is
decided here is what an answer means: nothing passes the write, `false`
refuses it, and a handler that raises or overruns refuses it too. A
guardrail that fails open is useless exactly when something is wrong.
EOF
)"
```

---

### Task 2: The `send` command asks before it writes

**Files:**
- Modify: `crates/neotmux/src/plugins.rs`
- Modify: `crates/neotmux/src/command.rs`
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: `LuaSubsystem::{has_write_hooks, dispatch_write}`, `WriteVerdict` from Task 1.
- Produces:
  ```rust
  impl Plugins {
      /// The one place a write is checked. `Ok(())` is a write that may go ahead; `Err` is
      /// the sentence the caller is told, already notified and logged.
      pub fn check_write(
          &mut self,
          session: SessionId,
          pane: PaneId,
          text: &str,
          from: &str,
      ) -> Result<(), String>;
  }
  ```

- [ ] **Step 1: Write the failing socket test**

In `crates/neotmux/tests/socket.rs`, following `a_plugin_can_write_into_a_pane` for the layer
and `send_reaches_the_pane_and_capture_reads_it_back` for driving the CLI:

```rust
/// The agent path, guarded: a plugin refuses the write, the command fails saying why, and
/// the shell never saw it.
#[test]
fn a_plugin_can_refuse_a_write_from_the_command_line() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[(
        "init.lua",
        "neotmux.on_write(function(write)
           if write.text:find('BLOCKED') then
             return false, 'BLOCKED is blocked'
           end
         end)",
    )]);
    let client = Client::start_with(runtime.path(), "/bin/cat", 24, 80, &[layer.path()]);
    wait_for_socket(&socket_in(runtime.path()));

    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["send", "%1", "BLOCKED-LINE", "--enter"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run send");
    assert!(!refused.status.success());
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(said.contains("BLOCKED is blocked"), "{said}");

    // And the one that is not blocked still arrives, so the refusal is a decision rather
    // than the path being broken.
    let sent = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["send", "%1", "ALLOWED-LINE", "--enter"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run send");
    assert!(sent.status.success(), "{}", String::from_utf8_lossy(&sent.stderr));
    client.wait_for_output("ALLOWED-LINE");
    drop(client);
}

/// Fail closed, end to end: the handler is broken, so the write does not happen.
#[test]
fn a_write_handler_that_raises_refuses_the_write() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[(
        "init.lua",
        "neotmux.on_write(function() error('the guard is broken') end)",
    )]);
    let client = Client::start_with(runtime.path(), "/bin/cat", 24, 80, &[layer.path()]);
    wait_for_socket(&socket_in(runtime.path()));

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["send", "%1", "NEVER-ARRIVES", "--enter"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run send");
    drop(client);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("the guard is broken"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux --test socket a_plugin_can_refuse 2>&1 | tail -20`
Expected: FAIL — the send succeeds, because nothing asks.

- [ ] **Step 3: Give the host one entry point**

In `crates/neotmux/src/plugins.rs`:

```rust
    /// The one place a write is checked, and the one place a refusal is announced.
    ///
    /// With nothing listening this is a registry lookup and no more, which is what keeps a
    /// server whose layers register no handler on exactly the path it has today.
    pub fn check_write(
        &mut self,
        session: SessionId,
        pane: PaneId,
        text: &str,
        from: &str,
    ) -> Result<(), String> {
        if !self.lua.has_write_hooks() {
            return Ok(());
        }
        match self.lua.dispatch_write(session, pane, text, from) {
            WriteVerdict::Pass => Ok(()),
            WriteVerdict::Refused(why) => {
                let said = format!("write to {pane} refused: {why}");
                logging::log(Level::Warn, "neotmux", &said);
                self.notifications.push(Level::Warn, "neotmux", &said);
                Err(why)
            }
        }
    }
```

- [ ] **Step 4: Ask from `send`**

In `crates/neotmux/src/command.rs`, in `send`, after the pane has been found and the bytes
built, and before `write_to_pane`. `Ctx` already carries `plugins`; it is `&Plugins` today and
`check_write` needs `&mut`, so widen the field to `&'a mut Plugins` and fix the one construction
site in `server.rs::answer`.

```rust
    for id in ctx.sessions.ids() {
        let holds = match ctx.sessions.get_mut(id) {
            Some(session) => session.holds_pane(pane),
            None => continue,
        };
        if !holds {
            continue;
        }
        // Asked once the owner is known, so a refusal names a pane that exists and a write
        // to a pane that does not is still the "no pane" answer it was.
        if let Err(why) = ctx.plugins.check_write(id, pane, &text, "tool") {
            return Ran::failed(why);
        }
        let Some(session) = ctx.sessions.get_mut(id) else {
            continue;
        };
        return match session.write_to_pane(pane, &bytes) {
            Ok(_) => Ran::ok(Vec::new()),
            Err(err) => Ran::failed(format!("could not write to %{}: {err:#}", pane.get())),
        };
    }
    Ran::failed(format!("no pane %{}", pane.get()))
```

`Session::holds_pane(&self, pane: PaneId) -> bool` is a new one-liner in `session.rs` beside
`write_to_pane` — `self.windows.all_panes().any(|held| held == pane)` — so the owner can be
found without writing first.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux --test socket && cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux
git commit -m "$(cat <<'EOF'
an agent's write is offered to the plugins first

`neotmux send` is the path an agent drives a pane through, and it went
straight to the pty. Now it is offered to whatever registered `on_write`,
and a refusal is the command's failure, a notification on the person's
screen, and a line in the log.

With nothing registered the path is unchanged: the question is a registry
lookup, not a trip into Lua.
EOF
)"
```

---

### Task 3: A plugin's own write passes the same hook

**Files:**
- Modify: `crates/neotmux/src/plugins.rs`
- Modify: `docs/superpowers/specs/2026-09-24-writes-pass-a-hook-design.md` (the `from` values)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: `Plugins::check_write` from Task 2.

`Op::PaneSend` is converted to `SessionOp::PaneSend` in `Plugins::respond`'s op drain, which
runs after `dispatch_tick` has returned — so the VM is idle there and a handler is never
re-entered from inside the tick that queued the write. That conversion is where the check goes:
a refused write becomes no op at all.

`from` for this path is `'plugin'`. The spec's §2 table says `'tool' | 'terminal'`; amend it to
`'tool'` (a command connection), `'plugin'` (a plugin's own `pane.send`), and say that
`'terminal'` is not emitted today because a person's keys are not writes.

- [ ] **Step 1: Write the failing socket test**

```rust
/// The bypass that would make all of this pointless: a plugin writing into a pane must pass
/// the same hook every other write passes.
#[test]
fn a_plugin_s_own_write_passes_the_hook_too() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[(
        "init.lua",
        "neotmux.on_write(function(write)
           neotmux.log.info('offered ' .. write.from .. ': ' .. write.text)
           if write.text:find('BLOCKED') then return false, 'no' end
         end)
         local sent = false
         neotmux.on_tick(function()
           if sent then return end
           local pane = neotmux.pane.current()
           if not pane then return end
           sent = true
           neotmux.pane.send(pane, 'BLOCKED-BY-THE-HOOK\\n')
         end)",
    )]);
    let client = Client::start_with(runtime.path(), "/bin/cat", 24, 80, &[layer.path()]);
    wait_for_log(runtime.path(), "offered plugin: BLOCKED-BY-THE-HOOK");
    wait_for_log(runtime.path(), "refused");
    drop(client);
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --test socket a_plugin_s_own_write 2>&1 | tail -20`
Expected: FAIL — nothing logs "offered plugin", because the op is converted without asking.

- [ ] **Step 3: Check it at the conversion**

In `Plugins::respond`'s op loop, replace the `Op::PaneSend` arm:

```rust
                Op::PaneSend { pane, text } => {
                    // After the tick has returned, so the VM is idle: a handler is never
                    // re-entered from inside the tick that queued this.
                    if self.check_write(session, pane, &text, "plugin").is_err() {
                        continue;
                    }
                    SessionOp::PaneSend { pane, text }
                }
```

Mind the borrow: the loop is inside a method that already holds `&mut self` — if the op drain
borrows `self.lua` immutably across the loop, collect the ops first and convert them after,
rather than fighting the borrow checker with a cell.

- [ ] **Step 4: Amend the spec's `from` table**

In `docs/superpowers/specs/2026-09-24-writes-pass-a-hook-design.md` §2, the `from` row becomes:

```
| `from` | `'tool'` for a connection that is not a client, `'plugin'` for a plugin's own
  `pane.send`. A person's keys are not writes (§1), so `'terminal'` is not emitted today. |
```

and §3's second bullet says the plugin path carries `'plugin'`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux --test socket && cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux docs
git commit -m "$(cat <<'EOF'
a plugin's write is offered to the plugins too

Hooking only the command line would make a plugin the way round it. A
`pane.send` is checked where its op is converted, after the tick that
raised it has returned, so a handler is never re-entered from inside the
tick that queued the write.
EOF
)"
```

---

### Task 4: The convention, and what it does not cover

**Files:**
- Create: `runtime/lua/neotmux/guard.lua`
- Modify: `docs/plugins.md`
- Test: `crates/neotmux-lua/tests/writes.rs`

**Interfaces:**
- Consumes: `neotmux.on_write` from Task 1.
- Produces: `require('neotmux.guard')` with `block(pattern, why)`, `only(from)`, `patterns`.

The host has never heard of any of this — it is a module in the same spirit as
`neotmux.mode`, `neotmux.box` and `neotmux.prompt`.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux-lua/tests/writes.rs`, with the shipped runtime layer on the path the way
`tests/copy.rs` puts it there (copy that harness exactly):

```rust
/// The convention on top of the hook: a pattern table, and one handler for all of it.
#[test]
fn the_guard_convention_refuses_a_line_that_matches_a_pattern() {
    let (_dir, sub) = shipped(&[(
        "init.lua",
        "local guard = require('neotmux.guard')
         guard.block('rm%s+%-%w*[rf]', 'rm -rf is blocked')",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "rm -rf /tmp/x\n", "tool"),
        WriteVerdict::Refused("rm -rf is blocked".to_string())
    );
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "ls -l\n", "tool"),
        WriteVerdict::Pass
    );
}

/// `only` is what makes a rule an agent's rule rather than everyone's.
#[test]
fn a_guard_can_apply_to_one_kind_of_writer() {
    let (_dir, sub) = shipped(&[(
        "init.lua",
        "local guard = require('neotmux.guard')
         guard.only('tool')
         guard.block('careful', 'not from a tool')",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "careful\n", "tool"),
        WriteVerdict::Refused("not from a tool".to_string())
    );
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "careful\n", "plugin"),
        WriteVerdict::Pass
    );
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test writes the_guard 2>&1 | tail -20`
Expected: FAIL — `module 'neotmux.guard' not found`.

- [ ] **Step 3: Write the module**

`runtime/lua/neotmux/guard.lua`:

```lua
-- Blocking a write by what it says, on top of `neotmux.on_write`.
--
-- The host decides that a handler which refuses, raises or overruns stops the write. *What* is
-- worth stopping is an opinion, and an opinion belongs here — beside `neotmux.mode` and
-- `neotmux.box` — rather than in the host. Replace this file with your own and nothing about
-- neotmux changes.
--
--   local guard = require('neotmux.guard')
--   guard.only('tool')                                 -- agents, not the person's plugins
--   guard.block('rm%s+%-%w*[rf]', 'rm -rf is blocked')
--
-- One handler is registered, the first time a rule is added: a table of patterns read on every
-- write, so adding a rule later needs no new subscription.
local M = {}

-- Lua patterns, each with what to say when it matches. Read on every write, so a rule added
-- from your own init.lua after load takes effect.
M.patterns = {}

-- Which writers the rules apply to, as `neotmux.on_write`'s `from`. `nil` is all of them.
M.from = nil

local registered = false

local function answer(write)
  if M.from and write.from ~= M.from then
    return
  end
  for _, rule in ipairs(M.patterns) do
    if write.text:find(rule.pattern) then
      return false, rule.why
    end
  end
end

local function arm()
  if registered then
    return
  end
  registered = true
  neotmux.on_write(answer)
end

-- A pattern to refuse, and the sentence whoever wrote it is told.
function M.block(pattern, why)
  M.patterns[#M.patterns + 1] = { pattern = pattern, why = why or ("'" .. pattern .. "' is blocked") }
  arm()
end

-- Narrows every rule to one kind of writer: 'tool' for a connection that is not a client,
-- 'plugin' for a plugin's own write.
function M.only(from)
  M.from = from
  arm()
end

return M
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --test writes`
Expected: PASS.

- [ ] **Step 5: Document it**

In `docs/plugins.md`, beside the subscription table at the top (`neotmux.on_key(fn)` etc.), add:

```
| `neotmux.on_write(fn)` | before text is written into a pane; return `false, why` to refuse it |
```

and a section after the pane API, titled **Guarding a write**, saying in the file's own voice:

- What it covers: `neotmux send` from outside and a plugin's `pane.send`.
- What it does not: a person's keystrokes (that is `on_key`, and the text there has to be read
  off the screen rather than known), reads, and an agent that does not use neotmux at all —
  "enforcing panes-only belongs to the agent's configuration or a sandbox; what this offers is
  a checkpoint on the writes that do come through".
- That everything which is not a pass is a refusal, including a handler that raises or
  overruns, and why: a guardrail that fails open is useless exactly when something is wrong.
- That with nothing registered the path is unchanged.
- The `neotmux.guard` convention, with the three-line example from the module's own header.

- [ ] **Step 6: Run the whole suite**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add runtime docs crates
git commit -m "$(cat <<'EOF'
a guard is a lua convention, not a host feature

`neotmux.guard` is a pattern table over one `on_write` handler, in the same
spirit as `neotmux.mode` and `neotmux.box`: the host never learns what any
of it means, and a user who replaces the file changes nothing about
neotmux.

The docs say what this does not cover, which matters more here than
anywhere else: a person's keys are `on_key`'s, and an agent that never
touches a pane is not neotmux's to stop.
EOF
)"
```

---

## Self-Review

**Spec coverage:** §2's primitive and its answers are Task 1; §3's two paths are Tasks 2 and 3;
§4's "at the door, inline" and "nothing registered, nothing changes" are Task 2's `check_write`
and Task 1's `has_write_hooks`; §5's fail-closed table is Task 1's dispatch plus Task 2's
end-to-end test; §7's three audiences are Task 2's `check_write`; §8's convention is Task 4.

**Not covered, deliberately:** §6's privilege rule. The connection kind is plumbed as `from`
(Tasks 2 and 3), but no command in the tree changes what neotmux does, so there is nothing to
refuse a `tool` connection yet. The rule is written in the spec; the first command that needs
it brings it. A task that implemented a refusal with nothing to refuse would be a task with no
test.

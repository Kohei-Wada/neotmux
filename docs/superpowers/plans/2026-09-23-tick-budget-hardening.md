# Tick Budget Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Lua tick budget actually able to stop a plugin — `pcall` can no longer swallow the interrupt, and key dispatch, event dispatch and config load are measured rather than unbounded.

**Architecture:** Three changes to `neotmux-lua`, all around `crate::budget`. The interrupt is made to survive `pcall` by sealing `pcall`, `xpcall` and `coroutine.resume` behind wrappers the plugin cannot get underneath — the trick `vm.rs` already uses for `load`. `Budget::enter` starts taking an explicit deadline so the same machinery can measure regions with different sizes, and the three unmeasured regions get wrapped.

**Tech Stack:** Rust, mlua 0.12 (Lua 5.4), `cargo test`.

**Spec:** `docs/superpowers/specs/2026-09-23-lua-on-the-server-design.md` §4

## Global Constraints

- This plan implements §4 of the spec and nothing else. The Lua layer does **not** move to the server here; no file outside `crates/neotmux-lua/` and `docs/plugins.md` changes.
- English for code, comments, docs and commit messages.
- Comments only where the WHY is non-obvious; one line where possible. Do not restate what the code says.
- No backward-compatibility shims, no `_`-prefixed unused bindings, no `// removed` comments. Delete outright.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` must all pass at the end of every task.
- `tick_budget_ms = 0` continues to switch off **all** measurement, including the new regions. It is the documented debugger escape hatch.
- Never claim a test passes without running it and reading the output.

---

### Task 1: The interrupt survives `pcall`

**Files:**
- Modify: `crates/neotmux-lua/src/budget.rs`
- Modify: `crates/neotmux-lua/src/lib.rs:112` (call site, in `LuaSubsystem::build`)
- Test: `crates/neotmux-lua/src/budget.rs` (the existing `mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub fn Budget::seal(lua: &Lua, budget: &Rc<Budget>) -> mlua::Result<()>`. Later tasks do not call it; it is called once at VM construction.

**Background.** The budget's hook raises an ordinary Lua error, so a plugin's `pcall` catches it and the loop goes round again. This is currently documented in `docs/plugins.md` as a known limitation ("An unbounded `while true do pcall(f) end` does hang neotmux"), not an unknown bug. Measured: against a 5 ms budget such a loop ran 892 ms without being interrupted.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/neotmux-lua/src/budget.rs`. Note these have **no escape hatch of their own** — if the seal fails they hang rather than fail, which is the point: a test that spins for ever is a regression you cannot miss.

```rust
    /// Sealing the globals is what makes the budget an interrupt rather than an error:
    /// without it a loop that calls anything inside `pcall` catches the budget's own error
    /// and goes round again, for ever.
    fn sealed(tick_budget_ms: u64) -> (Lua, Rc<Budget>) {
        let (lua, budget) = armed(tick_budget_ms);
        Budget::seal(&lua, &budget).unwrap();
        (lua, budget)
    }

    fn spins_through(lua: &Lua, body: &str) -> Function {
        lua.load(format!("return function() while true do {body} end end"))
            .eval()
            .unwrap()
    }

    #[test]
    fn a_sealed_pcall_cannot_swallow_the_budget() {
        let (lua, budget) = sealed(5);
        let spin = spins_through(&lua, "pcall(function() end)");

        budget.enter();
        let result = spin.call::<()>(());
        let outcome = budget.leave();

        assert!(result.is_err(), "pcall swallowed the budget error");
        assert_eq!(outcome, Outcome::Interrupted);
    }

    #[test]
    fn a_sealed_xpcall_cannot_swallow_the_budget() {
        let (lua, budget) = sealed(5);
        let spin = spins_through(&lua, "xpcall(function() end, function(e) return e end)");

        budget.enter();
        let result = spin.call::<()>(());
        budget.leave();

        assert!(result.is_err(), "xpcall swallowed the budget error");
    }

    #[test]
    fn a_sealed_resume_cannot_swallow_the_budget() {
        let (lua, budget) = sealed(5);
        let spin = spins_through(
            &lua,
            "local co = coroutine.create(function() end) coroutine.resume(co)",
        );

        budget.enter();
        let result = spin.call::<()>(());
        budget.leave();

        assert!(result.is_err(), "coroutine.resume swallowed the budget error");
    }

    #[test]
    fn a_pcall_inside_a_coroutine_cannot_swallow_the_budget() {
        let (lua, budget) = sealed(5);
        let spin: Function = lua
            .load(
                "return function()
                   coroutine.wrap(function()
                     while true do pcall(function() end) end
                   end)()
                 end",
            )
            .eval()
            .unwrap();

        budget.enter();
        let result = spin.call::<()>(());
        budget.leave();

        assert!(result.is_err(), "a coroutine's pcall swallowed the budget error");
    }

    /// The seal only bites once the budget has fired. An ordinary failure inside a tick is
    /// still an ordinary `false, err` — a plugin that guards its own work with `pcall` keeps
    /// working.
    #[test]
    fn a_sealed_pcall_still_reports_an_ordinary_failure() {
        let (lua, budget) = sealed(5_000);
        let probe: Function = lua
            .load("return function() return pcall(function() error('nope') end) end")
            .eval()
            .unwrap();

        budget.enter();
        let (ok, message) = probe.call::<(bool, String)>(()).unwrap();
        budget.leave();

        assert!(!ok);
        assert!(message.contains("nope"), "the error value was lost: {message}");
    }

    /// Multiple returns and the vararg shape survive the wrapper.
    #[test]
    fn a_sealed_pcall_passes_arguments_and_returns_through() {
        let (lua, budget) = sealed(5_000);
        let probe: Function = lua
            .load("return function() return pcall(function(a, b) return b, a end, 1, 2) end")
            .eval()
            .unwrap();

        budget.enter();
        let (ok, first, second) = probe.call::<(bool, i64, i64)>(()).unwrap();
        budget.leave();

        assert!(ok);
        assert_eq!((first, second), (2, 1));
    }

    /// A plugin that clears `table.pack` must not be able to break the seal with it.
    #[test]
    fn the_seal_does_not_depend_on_globals_a_plugin_can_replace() {
        let (lua, budget) = sealed(5);
        lua.load("table.pack = nil table.unpack = nil error = nil")
            .exec()
            .unwrap();
        let spin = spins_through(&lua, "pcall(function() end)");

        budget.enter();
        let result = spin.call::<()>(());
        budget.leave();

        assert!(result.is_err(), "the seal was broken by clearing a global");
    }

    /// An overrun that is not cleared would make the next unrelated `pcall` re-raise, outside
    /// any measured region. `leave` is where it is forgotten.
    #[test]
    fn leaving_forgets_the_overrun() {
        let (lua, budget) = sealed(5);
        let spin = spins_through(&lua, "pcall(function() end)");

        budget.enter();
        let _ = spin.call::<()>(());
        assert_eq!(budget.leave(), Outcome::Interrupted);

        assert!(!budget.overran(), "the overrun outlived the region it belonged to");
    }
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --lib budget::tests 2>&1 | tail -20`

Expected: the new tests fail to compile, because `Budget::seal` does not exist. That is the failure. Do **not** run the spin tests against an unsealed VM — without the seal they do not fail, they hang.

- [ ] **Step 3: Clear the overrun when the region closes**

In `crates/neotmux-lua/src/budget.rs`, `leave` currently reads the flag and leaves it set; `enter` is the only place that clears it. Once `pcall` consults the flag, a flag that outlives its region would make an unrelated failure re-raise. Take it rather than read it:

```rust
    /// Closes the slice and says what it came to. The elapsed comparison is what catches a
    /// tick the hook never got to sample past the deadline.
    ///
    /// The flag is taken, not read: the sealed `pcall` consults it, so an overrun that
    /// outlived its region would re-raise an unrelated failure in the next one.
    pub fn leave(&self) -> Outcome {
        let active = self.active.take();
        if self.overran.replace(false) {
            return Outcome::Interrupted;
        }
        match active {
            Some(active) => {
                let elapsed = active.start.elapsed();
                if elapsed > active.budget {
                    Outcome::RanLong { elapsed }
                } else {
                    Outcome::WithinBudget
                }
            }
            None => Outcome::WithinBudget,
        }
    }
```

`dispatch_tick` reads `budget.overran()` inside its callback loop, before `leave`, and uses the returned `Outcome` afterwards — nothing reads the flag after the region closes, so this is safe.

- [ ] **Step 4: Add the seal**

In `crates/neotmux-lua/src/budget.rs`, above `impl Budget`:

```rust
/// The globals a plugin could otherwise use to catch the budget's interrupt, replaced with
/// wrappers that re-raise it. The originals are locals in this chunk and the plugin has no
/// name for them, which is the trick `vm::SEAL` uses for `load`.
///
/// It delegates to the real `pcall` rather than reimplementing it, so the semantics a plugin
/// sees — callable values, varargs, multiple returns, message handlers — are exactly Lua's.
/// `table.pack`, `table.unpack` and `error` are captured too: a plugin that clears them must
/// not be able to break the seal with them.
const SEAL: &str = r#"
local overran = ...
local _pcall, _xpcall, _resume = pcall, xpcall, coroutine.resume
local _pack, _unpack, _error = table.pack, table.unpack, error

-- Level 0: the budget's message is the host's, and must not be prefixed with this chunk.
local function guard(r)
  if not r[1] and overran() then _error(r[2], 0) end
  return _unpack(r, 1, r.n)
end

pcall = function(...) return guard(_pack(_pcall(...))) end
xpcall = function(...) return guard(_pack(_xpcall(...))) end
coroutine.resume = function(...) return guard(_pack(_resume(...))) end
"#;
```

and, as a method on `Budget`:

```rust
    /// Makes the budget an interrupt rather than an error a plugin can catch. Called once,
    /// when the VM is built; the wrappers stay for the VM's life and cost nothing until a
    /// plugin calls one.
    pub fn seal(lua: &Lua, budget: &Rc<Budget>) -> mlua::Result<()> {
        let fired = budget.clone();
        let overran = lua.create_function(move |_, ()| Ok(fired.overran.get()))?;
        lua.load(SEAL)
            .set_name("=[neotmux budget seal]")
            .set_mode(mlua::chunk::ChunkMode::Text)
            .call::<()>(overran)
    }
```

The host function reaches the chunk as its vararg and is never set on the globals table, so a plugin cannot replace it.

- [ ] **Step 5: Call it when the VM is built**

In `crates/neotmux-lua/src/lib.rs`, at the `Budget::new` line in `LuaSubsystem::build`:

```rust
        let overrun = Rc::new(Overrun::new());
        let budget = Budget::new(opts.clone());
        Budget::seal(&lua, &budget).map_err(|err| anyhow::anyhow!("{err:#}"))?;
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p neotmux-lua 2>&1 | tail -20`
Expected: PASS, including the four spin tests. A hang here means the seal is not working — kill it and fix the seal rather than the test.

Run: `cargo test --workspace --no-fail-fast 2>&1 | tail -30`
Expected: every suite ok. `docs/plugins.md` still claims the old behaviour; Task 4 fixes that.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux-lua/src/budget.rs crates/neotmux-lua/src/lib.rs
git commit -m "the budget stops being something a plugin can catch"
```

---

### Task 2: Key and event dispatch are measured

**Files:**
- Modify: `crates/neotmux-lua/src/budget.rs` (`enter` takes a deadline)
- Modify: `crates/neotmux-lua/src/lib.rs` (`dispatch_tick`, `dispatch_key`, `dispatch_event`)
- Test: `crates/neotmux-lua/src/budget.rs`, and `crates/neotmux-lua/tests/budget.rs` for the dispatch-level behaviour — that file already holds the budget's integration tests and the helpers these need (`built`, `frame`, `logs`)

**Interfaces:**
- Consumes: `Budget::seal` from Task 1 — without it these tests hang instead of failing.
- Produces: `pub fn Budget::enter(&self, budget: Duration)`. `Budget::enabled` is unchanged and still keyed on `tick_budget_ms != 0`.

**Background.** `Budget::enter`/`leave` wrap tick dispatch and nothing else. The module says so: "key handling, event dispatch and config load are not measured against any budget." A `while true do end` in a key handler hangs neotmux today.

**Ruling, made here so the implementer does not have to guess:** an overrunning key or event handler is **cut off and reported, but does not count toward parking**. Parking removes a source's *tick* subscription and announces "is no longer ticking"; making a key handler trigger that would change a user-visible recovery story this plan has no mandate to change. Stopping the hang is the goal. If parking on key overruns turns out to be wanted, it is a later decision with its own reasoning.

- [ ] **Step 1: Write the failing test for the signature**

In `crates/neotmux-lua/src/budget.rs` tests, change every existing `budget.enter()` call to `budget.enter(Duration::from_millis(tick_budget_ms))` — the helpers already know the number. Add:

```rust
    /// The deadline is the caller's, so one VM can measure regions of different sizes.
    #[test]
    fn a_region_is_measured_against_the_deadline_it_was_given() {
        let (lua, budget) = sealed(5);
        let spin = spins_through(&lua, "pcall(function() end)");

        budget.enter(Duration::from_millis(200));
        let started = Instant::now();
        let result = spin.call::<()>(());
        let took = started.elapsed();
        budget.leave();

        assert!(result.is_err());
        assert!(
            took >= Duration::from_millis(200),
            "a 200 ms region was cut off after {took:?}"
        );
    }
```

Note the helpers pass `tick_budget_ms` into `Opts` as well, because `enabled()` still keys off it.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p neotmux-lua --lib budget::tests 2>&1 | tail -20`
Expected: fails to compile — `enter` takes no argument.

- [ ] **Step 3: Make the deadline the caller's**

In `crates/neotmux-lua/src/budget.rs`:

```rust
    /// Opens one measured region and starts its clock. Outside an `enter`/`leave` pair the
    /// hook does nothing.
    ///
    /// The deadline is the caller's because the regions differ: a tick gets a frame's worth,
    /// a config load gets far more. Whether anything is measured at all is still
    /// `tick_budget_ms`, which is the documented way to switch the budget off.
    ///
    /// A region is only measured if the frame armed a hook. A tick that raises the budget
    /// from 0 mid-frame would otherwise give the regions after it a deadline with nothing
    /// watching it: the loop that passes it is never interrupted and neotmux hangs.
    pub fn enter(&self, budget: Duration) {
        let measured = self.armed.get() && self.enabled();
        self.active.set(measured.then(|| Active {
            start: Instant::now(),
            budget,
        }));
        self.overran.set(false);
    }
```

In `dispatch_tick`, the one existing call site becomes:

```rust
            self.budget.enter(Duration::from_millis(self.opts.borrow().tick_budget_ms));
```

- [ ] **Step 4: Run the budget tests**

Run: `cargo test -p neotmux-lua --lib budget::tests 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Write the failing tests for the dispatch sites**

Append to `crates/neotmux-lua/tests/budget.rs`, which already has `built`, `frame` and `logs`.
These have no timeout of their own: without the fix they hang, and that hang is the failure.

```rust
/// A handler that never returns must not be able to hang neotmux, whichever entry point it
/// was reached through. Before this was measured, `dispatch_key` never came back.
#[test]
fn a_key_handler_that_loops_for_ever_is_cut_off() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.spin')"),
        (
            "lua/plugin/spin.lua",
            "neotmux.keymap.set('C-x', function() while true do end end) return {}",
        ),
    ]);

    let started = std::time::Instant::now();
    sub.dispatch_key("C-x");
    let took = started.elapsed();

    assert!(
        took < std::time::Duration::from_secs(2),
        "a spinning key handler ran for {took:?}"
    );
}

#[test]
fn an_event_handler_that_loops_for_ever_is_cut_off() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.spin')"),
        (
            "lua/plugin/spin.lua",
            "neotmux.on_event('pane_output', function() while true do end end) return {}",
        ),
    ]);

    let started = std::time::Instant::now();
    sub.dispatch_event(&neotmux_proto::Event::PaneOutput {
        pane: neotmux_proto::PaneId::from(1),
        bytes: b"hello".to_vec(),
    });
    let took = started.elapsed();

    assert!(
        took < std::time::Duration::from_secs(2),
        "a spinning event handler ran for {took:?}"
    );
}

/// The ruling for this task: an overrunning key handler is cut off and reported, and that is
/// all. Parking takes away a *tick* subscription and announces "is no longer ticking"; what
/// overran here was not a tick, so the source keeps drawing.
#[test]
fn a_key_handler_that_overruns_does_not_cost_the_source_its_tick() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.spin')"),
        (
            "lua/plugin/spin.lua",
            "neotmux.on_tick(function(ctx) ctx:text('still here', 0, 0) end)
             neotmux.keymap.set('C-x', function() while true do end end)
             return {}",
        ),
    ]);

    for _ in 0..8 {
        sub.dispatch_key("C-x");
    }
    frame(&sub);

    assert!(
        texts(&sub).iter().any(|t| t == "still here"),
        "a key overrun cost the source its tick"
    );
}
```

`texts` already exists in that file. Check `on_tick` and `ctx:text`'s exact shape against the
tests already in `tests/budget.rs` and match them rather than the sketch above if they differ.

- [ ] **Step 6: Run them to verify they fail**

Run: `timeout 60 cargo test -p neotmux-lua --test budget 2>&1 | tail -20`
Expected: the two spin tests do not finish, because nothing measures those regions yet. The
`timeout` is there so the run ends; that hang **is** the failure this task removes. Note it
and move on rather than waiting.

- [ ] **Step 7: Measure key dispatch**

In `crates/neotmux-lua/src/lib.rs`, wrap `dispatch_key`'s body. The hook is armed for the dispatch and taken away again, for the reason `dispatch_tick` already documents: an always-armed hook halves the interpreter's throughput.

```rust
    pub fn dispatch_key(&self, spec: &str) -> KeyOutcome {
        let Ok(normalised) = keymap::normalise(spec) else {
            return KeyOutcome::Fallthrough;
        };

        self.measured(|| {
            if self.offer_key(&normalised) == KeyOutcome::Consumed {
                return KeyOutcome::Consumed;
            }
            self.dispatch_binding(&normalised)
        })
    }
```

and add the helper beside it:

```rust
    /// One measured region outside a frame: the hook is armed for it and taken away again,
    /// and an overrun is reported against the source that was running. Unlike a tick, it does
    /// not count toward parking — parking takes away a tick subscription, and what overran
    /// here was not a tick.
    fn measured<T>(&self, run: impl FnOnce() -> T) -> T {
        if let Err(err) = Budget::arm(&self.lua, &self.budget) {
            self.report_text(source::ANONYMOUS, format!("the budget is off: {err}"));
        }
        let budget_ms = self.opts.borrow().tick_budget_ms;
        self.budget.enter(Duration::from_millis(budget_ms));
        let value = run();
        let outcome = self.budget.leave();
        self.budget.disarm(&self.lua);
        if outcome != Outcome::WithinBudget {
            self.report_text(
                source::ANONYMOUS,
                format!("a handler exceeded the {budget_ms} ms budget"),
            );
        }
        value
    }
```

The report is against `source::ANONYMOUS` because the callbacks in this region belong to different sources and the region is not attributable to one of them the way a tick group is. The per-callback error paths in `offer_one` and `dispatch_binding` already name the owner of whichever handler raised.

- [ ] **Step 8: Measure event dispatch**

Wrap `dispatch_event`'s dispatch loop in the same helper. The payload building above it is host work, not a plugin's, and stays outside the region.

- [ ] **Step 9: Run the tests**

Run: `timeout 120 cargo test -p neotmux-lua 2>&1 | tail -20`
Expected: PASS, and the budget suite completes rather than hanging.

Run: `cargo test --workspace --no-fail-fast 2>&1 | tail -30`
Expected: every suite ok.

- [ ] **Step 10: Commit**

```bash
git add crates/neotmux-lua
git commit -m "a key that never comes back is a key the budget cuts off"
```

---

### Task 3: Config load is measured against its own deadline

**Files:**
- Modify: `crates/neotmux-lua/src/api/opt.rs` (the new option)
- Modify: `crates/neotmux-lua/src/lib.rs` (`run_init`)
- Test: `crates/neotmux-lua/tests/budget.rs`

**Interfaces:**
- Consumes: `Budget::enter(Duration)` from Task 2.
- Produces: `Opts::config_budget_ms: u64`, default `5_000`, settable as `neotmux.opt.config_budget_ms`.

**Background.** `run_init` calls each layer's `init.lua` with nothing watching it. An `init.lua` that loops for ever hangs neotmux before a single frame is drawn. It cannot share the tick budget: a plugin's load legitimately does more work than a frame's worth, and being cut off at 8 ms would make plugins unloadable.

- [ ] **Step 1: Write the failing tests**

Append to `crates/neotmux-lua/tests/budget.rs`. The existing `built` asserts that loading was
clean, which is exactly what these tests are not, so they need a builder without that
assertion — the same shape `tests/keys.rs` uses. Add it beside `built_layers`:

```rust
/// `built_layers`, without its assertion that loading was clean: these tests are about a load
/// that is not.
fn built_loading_layers(layers: &[&[(&str, &str)]]) -> (Vec<tempfile::TempDir>, LuaSubsystem) {
    let mut dirs = Vec::new();
    let mut paths = Vec::new();
    for files in layers {
        let dir = tempfile::tempdir().unwrap();
        for (rel, body) in *files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        paths.push(dir.path().to_path_buf());
        dirs.push(dir);
    }
    let sub = LuaSubsystem::build(Layers::from_paths(paths), support::panes()).unwrap();
    (dirs, sub)
}
```

Check `built_layers` and copy its layer ordering rather than the sketch above: these tests need
the layer that sets the option to load *before* the one that spins, because a region's deadline
is fixed when it opens.

```rust
/// Two layers, because the deadline is fixed when the region opens: an `init.lua` cannot
/// lower its own budget, only the budget of the layers after it.
#[test]
fn an_init_that_loops_for_ever_is_cut_off_and_reported() {
    let (_dirs, sub) = built_loading_layers(&[
        &[("init.lua", "neotmux.opt.config_budget_ms = 50")],
        &[("init.lua", "while true do end")],
    ]);

    let started = std::time::Instant::now();
    let failures = sub.run_init();
    let took = started.elapsed();

    assert!(
        took < std::time::Duration::from_secs(5),
        "a spinning init.lua ran for {took:?}"
    );
    assert!(
        failures.iter().any(|f| f.message.contains("50 ms budget")),
        "the budget did not account for the load: {failures:?}"
    );
}

/// The reason config does not share the tick's deadline: this load is far past 8 ms and is
/// not a bug.
#[test]
fn a_slow_but_finite_init_still_loads() {
    let (_dirs, sub) = built_loading_layers(&[
        &[("init.lua", "neotmux.opt.config_budget_ms = 5000")],
        &[("init.lua", "local n = 0 for _ = 1, 20000000 do n = n + 1 end")],
    ]);

    assert!(
        sub.run_init().is_empty(),
        "a load well inside its budget was reported as a failure"
    );
}

/// Guard against the new `__newindex` arm swallowing typos.
#[test]
fn an_unknown_opt_key_is_still_refused() {
    let (_dirs, sub) = built_loading_layers(&[&[("init.lua", "neotmux.opt.config_budget = 50")]]);

    let failures = sub.run_init();

    assert!(
        failures.iter().any(|f| f.message.contains("config_budget")),
        "a misspelled option was accepted: {failures:?}"
    );
}
```

The default of 5000 ms is not asserted: proving it behaviourally costs a five-second test, and
the number is stated in `Opts::default` and in `docs/plugins.md` (Task 4).

- [ ] **Step 2: Run them to verify they fail**

Run: `timeout 60 cargo test -p neotmux-lua --test budget 2>&1 | tail -20`
Expected: the option is refused as an unknown key (`config_budget_ms` does not exist yet), and
the spinning-init test does not finish. The `timeout` ends the run; that hang is the bug.

- [ ] **Step 3: Add the option**

In `crates/neotmux-lua/src/api/opt.rs`, add the field to `Opts`:

```rust
    /// What one layer's `init.lua` gets. Far larger than a tick's: a load legitimately does
    /// more work than a frame, and cutting it off at the tick budget would make plugins
    /// unloadable. It is a deadline, not a target — anything approaching it is a bug in the
    /// plugin, and neotmux is unusable until it returns.
    pub config_budget_ms: u64,
```

default `config_budget_ms: 5_000`, and an arm in the `__newindex` function matching the shape of `tick_budget_ms`:

```rust
                "config_budget_ms" => {
                    let value = lua.unpack::<u64>(value)?;
                    opts.borrow_mut().config_budget_ms = value;
                }
```

Setting it from inside an `init.lua` affects the layers loaded after it, not the one setting it — the region is already open. That is the same rule `Budget::enter` already documents for a tick raising the budget mid-frame, and needs no new code.

- [ ] **Step 4: Measure the load**

In `crates/neotmux-lua/src/lib.rs`, `run_init`: arm once around the whole loop, and open a region per layer so that one slow layer does not spend another's deadline.

```rust
    pub fn run_init(&self) -> Vec<LoadFailure> {
        self.loading.set(true);
        if let Err(err) = Budget::arm(&self.lua, &self.budget) {
            self.report_text(source::ANONYMOUS, format!("the config budget is off: {err}"));
        }
        for layer in self.layers.paths() {
            // the existing body, unchanged, down to where `func` is built

            self.budget
                .enter(Duration::from_millis(self.opts.borrow().config_budget_ms));
            let called = func.call::<()>(());
            let outcome = self.budget.leave();
            if let Err(err) = called {
                self.load_failures.borrow_mut().push(LoadFailure {
                    source: tag(),
                    message: format!("{err:#}"),
                });
            }
            if outcome != Outcome::WithinBudget {
                let budget_ms = self.opts.borrow().config_budget_ms;
                self.load_failures.borrow_mut().push(LoadFailure {
                    source: tag(),
                    message: format!("loading exceeded its {budget_ms} ms budget"),
                });
            }
        }
        self.budget.disarm(&self.lua);
        self.loading.set(false);
        std::mem::take(&mut *self.load_failures.borrow_mut())
    }
```

An interrupted load produces two entries — the raised error and the budget's own account — which is the same pairing `dispatch_tick` produces and is what tells the author the error was not theirs.

- [ ] **Step 5: Run the tests**

Run: `timeout 120 cargo test -p neotmux-lua 2>&1 | tail -20`
Expected: PASS, nothing hangs.

Run: `cargo test --workspace --no-fail-fast 2>&1 | tail -30`
Expected: every suite ok. Watch the fixtures suite in particular — it loads real plugins through `run_init`.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux-lua
git commit -m "an init.lua that never returns is a load that fails, not a program that hangs"
```

---

### Task 4: The plugin documentation says what is true

**Files:**
- Modify: `docs/plugins.md`

**Interfaces:**
- Consumes: the behaviour of Tasks 1–3.
- Produces: nothing code-facing.

**Background.** `docs/plugins.md` is the user-facing plugin API reference. Three of its statements become false with Tasks 1–3 landed. It documented the `pcall` hole honestly, so the change is from one true statement to another.

- [ ] **Step 1: Correct the `pcall` bullet**

At `docs/plugins.md:849`, the bullet under "What the budget cannot cut off" currently reads:

```
- **`pcall`.** The budget error is an ordinary Lua error, so `pcall` catches it. It buys you
  nothing: you still lose the frame and the overrun is still counted against your limit. An
  unbounded `while true do pcall(f) end` does hang neotmux
```

`pcall` no longer belongs in a list of what cannot be cut off. Remove the bullet and state the new behaviour where the interrupt is described (around line 780, beside the note that a coroutine does not help): `pcall`, `xpcall` and `coroutine.resume` do not catch the budget's interrupt — once the deadline has passed they re-raise it — while an ordinary failure inside your own `pcall` is reported to you as it always was.

Leave the other three bullets — the long C call, `__gc` finalizers, and a blocking host binding — exactly as they are. They are still true, and the measured numbers in them were measured.

- [ ] **Step 2: Say what is measured**

The budget section describes a *tick* budget. Say that key handlers and event handlers are measured against the same number, and that an overrun there is cut off and logged but does not count toward the five — the parking story stays about ticks.

- [ ] **Step 3: Document the new option**

In the API list at `docs/plugins.md:638`, beside the existing two:

```lua
neotmux.opt.config_budget_ms = 5000  -- what one layer's init.lua gets; 0 turns the budget off
```

and say in the budget section that a load that passes it is reported as a load failure, naming the file, rather than hanging the program.

- [ ] **Step 4: Correct the escape-hatch paragraph**

At `docs/plugins.md:829`, "Turning it off" says a `while true do end` with the budget off "freezes neotmux with no way out but killing it". That is still true and stays. What changes is the reach of the sentence above it: with the budget on, the places that are now covered are ticks, key handlers, event handlers and config load — which is everywhere plugin code runs.

- [ ] **Step 5: Read the whole budget section back**

Read `docs/plugins.md` from the start of the budget section to the end of "What the budget cannot cut off" in one pass and check that it describes the code as it now is. This is a documentation task; the check is reading it, not running it.

- [ ] **Step 6: Commit**

```bash
git add docs/plugins.md
git commit -m "the plugin docs describe the budget that exists"
```

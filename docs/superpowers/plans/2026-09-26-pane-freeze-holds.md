# A freeze is a hold — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `neotmux.pane.freeze` returns a handle; a pane is frozen while any hold on it is
unreleased; a hold taken through `g.pane.freeze` is released when its group is emptied.

**Architecture:** `PaneTerminals` keeps a set of hold ids per pane and switches the existing
`TermState` snapshot on at the first hold and off at the last. `PaneRead` swaps
`freeze`/`unfreeze` for `hold`/`release`. The Lua binding wraps a `Hold` in a handle table; the
group machinery (`group.rs`, `Held`, `Sweep`) learns one more kind of thing a group holds.

**Tech Stack:** Rust (workspace crates `neotmux-proto`, `neotmux-pty`, `neotmux-lua`,
`neotmux-server`), mlua, Lua plugins under `runtime/lua`.

**Spec:** `docs/superpowers/specs/2026-09-26-pane-freeze-holds-design.md`

## Global Constraints

- Work in the worktree `.worktrees/pane-freeze-holds` (branch `pane-freeze-holds`), never the main checkout.
- One file = one type (`docs/superpowers/specs/2026-09-25-layout-one-file-per-type-design.md` §1): `Hold` gets its own file.
- `neotmux.pane.unfreeze` is removed outright — no compatibility shim.
- The wire is untouched; `PROTOCOL_VERSION` stays 15.
- Holds share the snapshot the first hold took; a later hold never re-takes the view.
- No release on Lua GC; no sweep of ungrouped holds on load failure or reload.
- Comments: default to none; one line only where the why is non-obvious; match the surrounding prose style where a doc comment is replaced.
- Before each commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets` clean, and the tests named in the task pass. The last task runs `cargo test --workspace`.
- Commit messages: a lowercase sentence describing the behaviour, as in `git log`, ending with the two attribution lines:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01TA1Zwx1euPs86d8MXxr6oo
  ```

## Review Focus

1. **A stale handle after a reload.** Old code's handle, released after the group already released it and new code took a fresh hold on the same pane, must answer `false` and leave the pane frozen — Task 3 test `an_old_handle_cannot_release_the_hold_that_replaced_it`.
2. **Copy mode or hints exiting while another holder remains.** The pane must stay frozen after the mode leaves — existing tests `copy.rs` (the `Z` key) and `hints.rs::a_pane_frozen_before_hints_opened_stays_frozen_after_hints_closes_any_way` keep covering it; they must pass unchanged in Task 2.
3. **`h.release()` called with a dot instead of a colon.** Should work the same — Task 2 test `a_freeze_from_lua_is_a_handle_that_releases_once`.
4. **Release after the pane closed.** Answers `false`, no panic — Task 1 test `a_hold_on_a_pane_that_closed_releases_nothing`.
5. **`unload` handler and group sweep both releasing on reload.** The second release is a no-op — existing `copy.rs::a_reload_with_copy_mode_open_leaves_no_pane_frozen` and `hints.rs::a_reload_with_a_pick_open_leaves_no_pane_frozen` must pass in Task 3.

---

### Task 1: `Hold` and `PaneTerminals::hold` / `release`

Adds the new host API beside the old one. The old `freeze`/`unfreeze` stay until Task 2 so the
workspace keeps building.

**Files:**
- Create: `neotmux-proto/src/screen/hold.rs`
- Modify: `neotmux-proto/src/screen/mod.rs`
- Modify: `neotmux-proto/src/screen/pane_read.rs`
- Modify: `neotmux-pty/src/pane_terminals.rs`

**Interfaces:**
- Produces:
  - `neotmux_proto::screen::Hold { pub pane: PaneId, id: u64 }` — `Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord`; `Hold::new(pane: PaneId, id: u64) -> Hold`; `Hold::id(&self) -> u64`
  - `PaneRead::hold(&self, pane: PaneId) -> Option<Hold>`
  - `PaneRead::release(&self, hold: Hold) -> bool`
  - `PaneTerminals::hold` / `PaneTerminals::release` with the same signatures

- [ ] **Step 1: Write the failing tests** in `neotmux-pty/src/pane_terminals.rs`'s `mod tests`, after `a_frozen_pane_that_closes_is_gone_like_any_other`:

```rust
    /// Two holders are independent: the pane moves again when the last one lets go, and
    /// until then it shows the screen the first hold took.
    #[test]
    fn a_pane_moves_again_only_when_its_last_hold_is_released() {
        let terminals = terminals();
        terminals.feed(pane(1), b"first");
        let a = terminals.hold(pane(1)).expect("a live pane");
        terminals.feed(pane(1), b"\r\nsecond");
        let b = terminals.hold(pane(1)).expect("a live pane");
        assert_eq!(
            lines(&terminals, 1),
            vec!["first", "", "", ""],
            "the second hold re-took the view"
        );

        assert!(terminals.release(a));
        assert_eq!(terminals.is_frozen(pane(1)), Some(true));
        assert_eq!(lines(&terminals, 1), vec!["first", "", "", ""]);

        assert!(terminals.release(b));
        assert_eq!(terminals.is_frozen(pane(1)), Some(false));
        assert_eq!(lines(&terminals, 1), vec!["first", "second", "", ""]);
    }

    #[test]
    fn a_hold_releases_once() {
        let terminals = terminals();
        let a = terminals.hold(pane(1)).expect("a live pane");
        let b = terminals.hold(pane(1)).expect("a live pane");
        assert!(terminals.release(a));
        assert!(!terminals.release(a), "a second release took somebody else's hold");
        assert_eq!(terminals.is_frozen(pane(1)), Some(true));
        assert!(terminals.release(b));
    }

    #[test]
    fn a_hold_on_a_pane_that_closed_releases_nothing() {
        let terminals = terminals();
        let a = terminals.hold(pane(1)).expect("a live pane");
        terminals.close(pane(1));
        assert!(!terminals.release(a));
    }

    #[test]
    fn holding_a_pane_nobody_has_answers_none() {
        assert_eq!(terminals().hold(pane(4242)), None);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p neotmux-pty pane_terminals`
Expected: compile error, `no method named hold found`.

- [ ] **Step 3: Create `neotmux-proto/src/screen/hold.rs`**

```rust
use crate::PaneId;

/// One caller's claim on a frozen pane. The pane stays frozen while any hold on it is
/// unreleased, so two callers freezing the same pane cannot thaw it for each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hold {
    pub pane: PaneId,
    id: u64,
}

impl Hold {
    /// For the implementor of [`PaneRead`](crate::screen::PaneRead): `id` is one it has
    /// never handed out before.
    pub fn new(pane: PaneId, id: u64) -> Self {
        Self { pane, id }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}
```

In `neotmux-proto/src/screen/mod.rs` add `mod hold;` after `mod frame;` and `pub use hold::*;`
after `pub use frame::*;`.

- [ ] **Step 4: Add to `PaneRead`** (`neotmux-proto/src/screen/pane_read.rs`): add
`use crate::screen::Hold;` to the imports and, after `fn is_frozen`:

```rust
    /// Holds the pane still until the hold is released: it draws and reads as it was when
    /// the first hold on it was taken. `None` for a pane that does not exist.
    fn hold(&self, pane: PaneId) -> Option<Hold>;
    /// `false` if this hold was already released or its pane is gone.
    fn release(&self, hold: Hold) -> bool;
```

- [ ] **Step 5: Implement in `PaneTerminals`** (`neotmux-pty/src/pane_terminals.rs`)

Imports: `use std::cell::{Cell, RefCell};`, `use std::collections::{BTreeMap, BTreeSet};`, and
`Hold` added to the `neotmux_proto::screen` import.

`Terminal` gains a field after `frozen_revision`:

```rust
    holds: BTreeSet<u64>,
```

`PaneTerminals` gains, after `panes`:

```rust
    next_hold: Cell<u64>,
```

`open` initialises `holds: BTreeSet::new(),`.

After `is_frozen`:

```rust
    pub fn hold(&self, pane: PaneId) -> Option<Hold> {
        let mut panes = self.panes.borrow_mut();
        let terminal = panes.get_mut(&pane)?;
        if terminal.holds.is_empty() && terminal.state.freeze() {
            terminal.frozen_revision = Some(terminal.revision);
        }
        let id = self.next_hold.get() + 1;
        self.next_hold.set(id);
        terminal.holds.insert(id);
        Some(Hold::new(pane, id))
    }

    pub fn release(&self, hold: Hold) -> bool {
        let mut panes = self.panes.borrow_mut();
        let Some(terminal) = panes.get_mut(&hold.pane) else {
            return false;
        };
        if !terminal.holds.remove(&hold.id()) {
            return false;
        }
        if terminal.holds.is_empty() {
            terminal.state.unfreeze();
            terminal.frozen_revision = None;
        }
        true
    }
```

In `impl PaneRead for PaneTerminals`, after `is_frozen`:

```rust
    fn hold(&self, pane: PaneId) -> Option<Hold> {
        PaneTerminals::hold(self, pane)
    }

    fn release(&self, hold: Hold) -> bool {
        PaneTerminals::release(self, hold)
    }
```

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p neotmux-pty pane_terminals`
Expected: all pass, including the four new tests.

- [ ] **Step 7: fmt, clippy, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets
git add neotmux-proto/src/screen neotmux-pty/src/pane_terminals.rs
git commit -m "a pane can be held by several callers at once, and moves again when the last one lets go" # plus attribution lines
```

---

### Task 2: the Lua handle, and `freeze`/`unfreeze` go

**Files:**
- Modify: `neotmux-proto/src/screen/pane_read.rs` (remove `freeze`, `unfreeze`)
- Modify: `neotmux-pty/src/pane_terminals.rs` (remove `freeze`, `unfreeze`, their trait impls; move three tests to `hold`/`release`)
- Modify: `neotmux-server/src/shells.rs:497,511`
- Modify: `neotmux-lua/src/api/pane.rs:240-256`
- Modify: `neotmux-lua/tests/freeze.rs`
- Modify: `runtime/lua/plugin/copy.lua`
- Modify: `runtime/lua/neotmux/hints.lua`

**Interfaces:**
- Consumes: `Hold`, `PaneRead::hold`, `PaneRead::release` (Task 1)
- Produces: Lua `neotmux.pane.freeze(id) -> handle | nil`, `handle.pane: integer`, `handle:release() -> boolean`; `fn hold_handle(lua: &Lua, terminals: Rc<dyn PaneRead>, hold: Hold) -> mlua::Result<Table>` private to `pane.rs`

- [ ] **Step 1: Rewrite the Lua-shape test** in `neotmux-lua/tests/freeze.rs`. Replace
`freezing_and_unfreezing_from_lua_says_whether_the_pane_was_there` with:

```rust
#[test]
fn a_freeze_from_lua_is_a_handle_that_releases_once() {
    let (_dir, sub) = built(
        "local h = neotmux.pane.freeze(1)\n\
         neotmux.log.info(tostring(h.pane)\n\
         .. ' ' .. tostring(neotmux.pane.freeze(4242))\n\
         .. ' ' .. tostring(neotmux.pane.frozen(1))\n\
         .. ' ' .. tostring(h:release())\n\
         .. ' ' .. tostring(h.release())\n\
         .. ' ' .. tostring(neotmux.pane.frozen(1))\n\
         .. ' ' .. tostring(neotmux.pane.frozen(4242))\n\
         .. ' ' .. tostring(neotmux.pane.unfreeze))",
    );
    sub.terminals().open(pane(1), SIZE, 100);
    assert_eq!(
        ticked(&sub),
        vec!["1 nil true true false false nil nil"]
    );
}
```

In the same file, move the other tests off the removed API:
- `the_display_stops_moving_while_the_pane_goes_on_being_fed`: `assert!(sub.terminals().freeze(pane(1)));` → `let hold = sub.terminals().hold(pane(1)).expect("a live pane");`; `assert!(sub.terminals().unfreeze(pane(1)));` → `assert!(sub.terminals().release(hold));`
- `a_frozen_pane_reads_as_the_text_it_was_frozen_on`: the same two replacements.
- `a_frozen_pane_still_reports_its_output`: `sub.terminals().freeze(pane(1));` → `sub.terminals().hold(pane(1));`

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p neotmux-lua --test freeze`
Expected: `a_freeze_from_lua_is_a_handle_that_releases_once` fails (the log line starts `nil` — `h.pane` of a boolean raises, so the tick errors and logs nothing; either way the assertion fails). The other three pass.

- [ ] **Step 3: Replace the Lua binding** in `neotmux-lua/src/api/pane.rs`. Add `Hold` to the
`neotmux_proto::screen` import. Replace the `freeze` and `unfreeze` blocks (lines ~240-256) with:

```rust
    {
        // Immediate, not an op: a plugin freezes a pane in order to read it in the same
        // call, and an op applied after the tick would hand it the live text it froze to
        // get away from. Nothing about the tree changes, so there is nothing for the rest of
        // the frame to catch up with.
        let terminals = terminals.clone();
        let freeze = lua.create_function(move |lua, pane: u32| {
            Ok(match terminals.hold(PaneId::from(pane)) {
                Some(hold) => Value::Table(hold_handle(lua, terminals.clone(), hold)?),
                None => Value::Nil,
            })
        })?;
        pane.set("freeze", freeze)?;
    }
```

And at the bottom of the file (outside `install`):

```rust
fn hold_handle(lua: &Lua, terminals: Rc<dyn PaneRead>, hold: Hold) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("pane", hold.pane.get())?;
    let release = lua.create_function(move |_, _self: Value| Ok(terminals.release(hold)))?;
    handle.set("release", release)?;
    Ok(handle)
}
```

- [ ] **Step 4: Remove the old host API.**
- `pane_read.rs`: delete `fn freeze(&self, pane: PaneId) -> bool;` and `fn unfreeze(&self, pane: PaneId) -> bool;`.
- `pane_terminals.rs`: delete the inherent `freeze` and `unfreeze` (with their doc comments) and the two trait-impl forwarders. Move the doc comment's content onto `hold`, adjusted:

```rust
    /// Holds the pane still: until every hold on it is released it draws, reads and numbers
    /// its history as it was when the first hold was taken, while its child's output goes on
    /// being parsed into the live screen underneath. A later hold shares that snapshot rather
    /// than taking another, so a caller that has anchored a selection to it does not lose it.
    /// `None` for a pane that does not exist.
```

and on `release`:

```rust
    /// Lets go of one hold; the pane catches up with everything that arrived while it was
    /// held once no hold is left. `false` if this hold was already released or its pane is
    /// gone.
```

- `pane_terminals.rs` tests:
  - `a_frozen_panes_revision_stands_still_and_catches_up_when_it_is_let_go`: `assert!(terminals.freeze(pane(1)));` → `let hold = terminals.hold(pane(1)).expect("a live pane");`; `assert!(terminals.unfreeze(pane(1)));` → `assert!(terminals.release(hold));`
  - delete `freezing_a_pane_nobody_has_says_so_rather_than_raising` (covered by `holding_a_pane_nobody_has_answers_none`; move its `assert_eq!(terminals.is_frozen(pane(4242)), None);` into that test).
  - `a_frozen_pane_that_closes_is_gone_like_any_other`: `terminals.freeze(pane(1));` → `terminals.hold(pane(1));`
- `neotmux-server/src/shells.rs`: `assert!(terminals.freeze(pane(1)));` → `let hold = terminals.hold(pane(1)).expect("a live pane");`; `assert!(terminals.unfreeze(pane(1)));` → `assert!(terminals.release(hold));`

- [ ] **Step 5: Move `plugin.copy`** (`runtime/lua/plugin/copy.lua`).

In `leave`, replace

```lua
  if state and state.froze then
    -- The pane goes back to showing what it is doing, and to moving while it does it — unless
    -- it was already being held still when the mode was entered, in which case whoever froze
    -- it is still using it and this is not the mode that thaws it.
    neotmux.pane.unfreeze(state.pane)
  end
```

with

```lua
  if state and state.hold then
    -- The pane goes back to showing what it is doing, unless somebody else is holding it too.
    state.hold:release()
  end
```

In the entering function, replace the comment ending `...whatever freeze it was holding, so the freeze this mode takes below is its own.` — its last sentence becomes:

```lua
  -- could be neither read for what they are nor typed at. Ending it first also lets go of the
  -- pane it held, so the freeze below takes this instant's screen rather than the pick's.
```

and replace

```lua
  -- Held still for the life of the mode. The buffer above is the text the cursor is dragged
  -- over; this is the same instant's cells, and the two have to be the same instant or the
  -- selection is over one thing and the screen shows another. `freeze` answers `true` for any
  -- pane that exists rather than for one it changed, so whether this mode is the one holding
  -- the pane still is a question only `frozen` can answer, and only before the freeze.
  local froze = not neotmux.pane.frozen(pane) and neotmux.pane.freeze(pane) or false
```

with

```lua
  -- Held still for the life of the mode. The buffer above is the text the cursor is dragged
  -- over; this is the same instant's cells, and the two have to be the same instant or the
  -- selection is over one thing and the screen shows another.
  local hold = neotmux.pane.freeze(pane)
```

and in the `state = { ... }` table, `froze = froze,` → `hold = hold,`.

- [ ] **Step 6: Move `neotmux.hints`** (`runtime/lua/neotmux/hints.lua`).

Header comment (lines ~28-30) becomes:

```lua
-- Every pane a target was found on is held still for the life of the session and let go on
-- the way out, whichever way it leaves. A pane somebody else was already holding (copy mode,
-- say) stays held by them.
```

In `close`, replace

```lua
    for pane in pairs(state.frozen) do
      neotmux.pane.unfreeze(pane)
    end
```

with

```lua
    for _, hold in ipairs(state.holds) do
      hold:release()
    end
```

In `open`, replace the comment and loop

```lua
  -- Frozen once per pane, and only where it was not already: copy mode, or an earlier hint
  -- session, may have gotten there first, and this session must not be the one that thaws it.
  -- `freeze` answers `true` for any pane that exists, frozen or not already — it is `frozen`
  -- that says which, and it has to be asked before `freeze` runs and makes the question moot.
  local frozen = {}
  local seen = {}
  for _, hit in ipairs(found) do
    if not seen[hit.pane] then
      seen[hit.pane] = true
      local already = neotmux.pane.frozen(hit.pane)
      if not already and neotmux.pane.freeze(hit.pane) then
        frozen[hit.pane] = true
      end
    end
  end
```

with

```lua
  local holds = {}
  local seen = {}
  for _, hit in ipairs(found) do
    if not seen[hit.pane] then
      seen[hit.pane] = true
      holds[#holds + 1] = neotmux.pane.freeze(hit.pane)
    end
  end
```

and in the `state = { ... }` table, `frozen = frozen,` → `holds = holds,`.

- [ ] **Step 7: Run the affected suites**

Run: `cargo test -p neotmux-pty && cargo test -p neotmux-lua --test freeze --test copy --test hints && cargo test -p neotmux-server shells`
Expected: all pass, with no change to any `copy.rs` or `hints.rs` test.

Then: `grep -rn "unfreeze\|\.froze\b\|state.frozen" runtime neotmux-lua/src neotmux-pty/src neotmux-proto/src neotmux-server/src`
Expected: only `TermState::unfreeze` and its own tests in `neotmux-pty/src/screen/term_state.rs`, and the call to it in `PaneTerminals::release`.

- [ ] **Step 8: fmt, clippy, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets
git add -A
git commit -m "pane.freeze hands back a hold that releases only itself, and pane.unfreeze is gone" # plus attribution lines
```

---

### Task 3: `g.pane.freeze` — a group releases what it held

**Files:**
- Modify: `neotmux-lua/src/plugin/held.rs`
- Modify: `neotmux-lua/src/api/sweep.rs`
- Modify: `neotmux-lua/src/api/install.rs` (the `Sweep` it builds; `pane::install` call)
- Modify: `neotmux-lua/src/lua_subsystem.rs` (a `panes` field; the `Sweep` it builds)
- Modify: `neotmux-lua/src/api/group.rs`
- Modify: `neotmux-lua/src/api/pane.rs` (note the hold)
- Modify: `runtime/lua/plugin/copy.lua`, `runtime/lua/neotmux/hints.lua`
- Test: `neotmux-lua/tests/freeze.rs`

**Interfaces:**
- Consumes: `Hold`, `PaneRead::release` (Task 1); the `freeze` binding and `hold_handle` (Task 2)
- Produces: `Held::Freeze(Hold)`; `Sweep { .., panes: Rc<dyn PaneRead> }`; Lua `g.pane.freeze(id)`; `pane::install(.., terminals: Rc<dyn PaneRead>, groups: Rc<Groups>)`

- [ ] **Step 1: Write the failing tests** in `neotmux-lua/tests/freeze.rs`.

Split the helper so a test can write the whole `init.lua`. Replace `built` with:

```rust
fn built(body: &str) -> (tempfile::TempDir, Built) {
    built_init(&format!("neotmux.on_tick(function()\n{body}\nend)\n"))
}

fn built_init(init: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), format!("{REPORT}\n{init}")).unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, Built::new(sub, terminals))
}
```

Add after it:

```rust
/// `F` takes a hold through the group, keeping the one before it as `old`; `N` names the
/// group again; `P` takes a hold outside any group; `O` releases `old`.
const GROUPED: &str = "local g = neotmux.group('t')\n\
     local held, old\n\
     neotmux.on_key(function(key)\n\
       if key == 'F' then old = held; held = g.pane.freeze(1); return true end\n\
       if key == 'N' then neotmux.group('t'); return true end\n\
       if key == 'P' then neotmux.pane.freeze(1); return true end\n\
       if key == 'O' then neotmux.log.info('old=' .. tostring(old:release())); return true end\n\
       return false\n\
     end)\n";
```

and the tests:

```rust
#[test]
fn a_hold_taken_through_a_group_is_released_when_the_group_is_named_again() {
    let (_dir, sub) = built_init(GROUPED);
    sub.terminals().open(pane(1), SIZE, 100);
    support::keys(&sub, &["F"]);
    assert_eq!(sub.terminals().is_frozen(pane(1)), Some(true));
    support::keys(&sub, &["N"]);
    assert_eq!(sub.terminals().is_frozen(pane(1)), Some(false));
}

#[test]
fn a_reload_releases_a_grouped_hold_and_leaves_an_ungrouped_one() {
    let (_dir, sub) = built_init(GROUPED);
    sub.terminals().open(pane(1), SIZE, 100);
    support::keys(&sub, &["F"]);
    sub.reload(Some(support::one()));
    assert_eq!(
        sub.terminals().is_frozen(pane(1)),
        Some(false),
        "the reload left a grouped hold in place"
    );
    support::keys(&sub, &["P"]);
    sub.reload(Some(support::one()));
    assert_eq!(
        sub.terminals().is_frozen(pane(1)),
        Some(true),
        "the reload released a hold no group owned"
    );
}

/// The handle the old code kept outlives the group that released it. Releasing it must not
/// take the hold the new code took since.
#[test]
fn an_old_handle_cannot_release_the_hold_that_replaced_it() {
    let (_dir, sub) = built_init(GROUPED);
    sub.terminals().open(pane(1), SIZE, 100);
    support::keys(&sub, &["F", "N", "F", "O"]);
    assert!(support::logged(&sub).iter().any(|line| line == "old=false"));
    assert_eq!(sub.terminals().is_frozen(pane(1)), Some(true));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p neotmux-lua --test freeze`
Expected: the three new tests fail — `g.pane` is nil, so `F` raises (`attempt to index a nil value`), and the frozen assertions fail.

- [ ] **Step 3: `Held::Freeze`** (`neotmux-lua/src/plugin/held.rs`): add
`use neotmux_proto::screen::Hold;` to the imports and, after `Timer(TimerId),`:

```rust
    Freeze(Hold),
```

- [ ] **Step 4: `Sweep` releases it** (`neotmux-lua/src/api/sweep.rs`): add
`use neotmux_proto::screen::PaneRead;`, a field after `commands`:

```rust
    pub(crate) panes: Rc<dyn PaneRead>,
```

and in `empty`'s match, after the `Held::Timer` arm:

```rust
                Held::Freeze(hold) => {
                    self.panes.release(hold);
                }
```

Update the doc comment on the struct: "the callers otherwise take seven collaborators" → "eight".

- [ ] **Step 5: Give both `Sweep` builders the panes.**
- `neotmux-lua/src/api/install.rs`: in the `let sweep = Sweep { .. }` literal add `panes: panes.clone(),`. In the `super::pane::install(..)` call pass `panes` as before and add `groups.clone()` as a new last argument.
- `neotmux-lua/src/lua_subsystem.rs`: add a field `panes: Rc<dyn PaneRead>,` to `LuaSubsystem` (after `groups`); in `build_with_runner` pass `panes: panes.clone(),` into `api::Shared` and set `panes,` in the `Ok(Self { .. })` literal; in `fn sweep` add `panes: self.panes.clone(),`.

- [ ] **Step 6: The binding notes the hold** (`neotmux-lua/src/api/pane.rs`): `install` takes a new
last parameter `groups: Rc<Groups>`; import `crate::plugin::{Groups, Held}` alongside the
existing `crate::plugin` import. In the `freeze` block, clone `groups` in and note the hold
before building the handle:

```rust
        let terminals = terminals.clone();
        let groups = groups.clone();
        let freeze = lua.create_function(move |lua, pane: u32| {
            Ok(match terminals.hold(PaneId::from(pane)) {
                Some(hold) => {
                    groups.note(Held::Freeze(hold));
                    Value::Table(hold_handle(lua, terminals.clone(), hold)?)
                }
                None => Value::Nil,
            })
        })?;
```

(`groups.note` files nothing outside a `g.*` call, so plain `neotmux.pane.freeze` is unaffected.)

- [ ] **Step 7: `g.pane`** (`neotmux-lua/src/api/group.rs`): `NESTED` becomes

```rust
const NESTED: [(&str, &[&str]); 5] = [
    ("keymap", &["set"]),
    ("command", &["add"]),
    ("job", &["start"]),
    ("timer", &["every", "after"]),
    ("pane", &["freeze"]),
];
```

- [ ] **Step 8: Run to verify they pass**

Run: `cargo test -p neotmux-lua --test freeze --test groups`
Expected: all pass.

- [ ] **Step 9: The shipped users take their holds through their groups.**
- `runtime/lua/plugin/copy.lua`: `local hold = neotmux.pane.freeze(pane)` → `local hold = g.pane.freeze(pane)`. Replace the comment above `g.on_event("unload", ...)`:

```lua
-- A reload runs this file again with a copy mode that has not entered anything. The group
-- lets go of the pane on its own; leaving is for the mode's layer and the pick.
```

- `runtime/lua/neotmux/hints.lua`: `holds[#holds + 1] = neotmux.pane.freeze(hit.pane)` → `holds[#holds + 1] = g.pane.freeze(hit.pane)`.

- [ ] **Step 10: Run the suites the shipped users are tested by**

Run: `cargo test -p neotmux-lua --test copy --test hints --test reload --test groups --test freeze`
Expected: all pass, including `copy.rs::a_reload_with_copy_mode_open_leaves_no_pane_frozen` and `hints.rs::a_reload_with_a_pick_open_leaves_no_pane_frozen`.

- [ ] **Step 11: fmt, clippy, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets
git add -A
git commit -m "a freeze taken through a group is let go when the group is emptied, and copy mode and hints take theirs that way" # plus attribution lines
```

---

### Task 4: docs and changelog

**Files:**
- Modify: `docs/plugins.md`
- Modify: `CHANGELOG.md`
- Modify: `runtime/lua/neotmux/snapshot.lua:10` only if its wording names `unfreeze` (it does not at the time of writing — check, leave alone otherwise)

**Interfaces:**
- Consumes: the Lua API as built in Tasks 2-3.

- [ ] **Step 1: The pane API list** (`docs/plugins.md`, ~line 380). Replace the three lines

```
neotmux.pane.freeze(id)       -- hold the pane still; false if there is no such pane
neotmux.pane.unfreeze(id)     -- let it catch up; false if there is no such pane
neotmux.pane.frozen(id)       -- true, false, or nil for no such pane
```

with

```
neotmux.pane.freeze(id)       -- hold the pane still: a handle, or nil if there is no such pane
neotmux.pane.frozen(id)       -- true while anyone holds it, false, or nil for no such pane
```

- [ ] **Step 2: "Holding a pane still"** (~line 568). Replace the code block

```lua
neotmux.pane.freeze(id)
-- the pane draws, reads and numbers its history as it was at this moment
neotmux.pane.unfreeze(id)
```

with

```lua
local hold = neotmux.pane.freeze(id)
-- the pane draws, reads and numbers its history as it was at this moment
hold:release()
```

Replace "Unfreezing shows everything that arrived." with "Releasing the last hold shows
everything that arrived."

Replace the "Four things worth knowing" first bullet (**A second `freeze` keeps the snapshot it
already has.** …) with:

```markdown
- **A freeze is a hold, and a pane is frozen while anyone holds it.** Two plugins freezing the
  same pane each get their own handle; one releasing does not move the pane under the other.
  `hold:release()` answers `true` the first time and `false` after, or once the pane is gone —
  it can only ever let go of its own hold. A second hold shares the snapshot the first one
  took rather than re-taking the view, since re-anchoring it would move a selection out from
  under whoever took one.
```

In the resize bullet, "unfreezing shows the reflowed screen" → "releasing shows the reflowed
screen" and "what it names after the unfreeze" → "what it names after the release".

After the bullets, add:

```markdown
A hold taken through your group — `g.pane.freeze(id)` — is released when the group is emptied,
so a reload never leaves a pane frozen by code that is no longer loaded. A hold from plain
`neotmux.pane.freeze` is yours to release, the way a timer armed outside a group is yours to
stop.
```

- [ ] **Step 3: The `ctx:view` example** (~line 1764). Replace

```lua
neotmux.pane.freeze(pane)          -- on the way in
ctx:view(pane, back)               -- every tick, from your own idea of where you are
neotmux.pane.unfreeze(pane)        -- on the way out
```

with

```lua
local hold = g.pane.freeze(pane)   -- on the way in
ctx:view(pane, back)               -- every tick, from your own idea of where you are
hold:release()                     -- on the way out
```

- [ ] **Step 4: The hints paragraph** (~line 1925). Replace from "say, got there first): that one
is left exactly as it was found" through "…because entering the mode calls
`neotmux.hints.cancel()` first:" so the paragraph reads:

```markdown
Every pane a target was found on is held still for the life of the session and let go on the
way out, whichever way it leaves. A pane somebody else was holding already — copy mode, say —
stays held by them: that is what lets `f` inside copy mode open a hint session over the very
pane copy mode is holding, without either one undoing the other. A copy mode entered over a live
hint session does not arise, because entering the mode calls `neotmux.hints.cancel()` first:
```

(keep the rest of that sentence — "the mode is taking the pane and the keyboard, …" — as is).
Read the surrounding lines before editing and keep the paragraph's first sentence if it already
says the first half; the goal is that the words "a flag and not a count" and "asks
`pane.frozen` before it freezes" are gone.

- [ ] **Step 5: The `unload` paragraph** (~line 2256). Replace

```markdown
`unload` is for what no group can see. A group knows what you registered, not what you did: a
pane you froze with `neotmux.pane.freeze` stays frozen after the code that meant to unfreeze it
is gone, unless you unfreeze it on the way out.

```lua
g.on_event('unload', function()
  for pane in pairs(frozen) do neotmux.pane.unfreeze(pane) end
end)
```
```

with

```markdown
`unload` is for what no group can see. A group knows what you registered through it, not
everything you did: a hold from plain `neotmux.pane.freeze` stays after the code that meant to
release it is gone, unless you release it on the way out — or take it with `g.pane.freeze`,
which the group releases for you.

```lua
g.on_event('unload', function()
  for _, hold in ipairs(holds) do hold:release() end
end)
```
```

Replace "The shipped copy mode and hints do exactly this." with "The shipped copy mode and
hints take their holds through their groups, and use `unload` for their modes."

- [ ] **Step 6: Anything missed**

Run: `grep -n "unfreeze\|flag and not a count\|second .freeze" docs/plugins.md README.md`
Expected: no output.

- [ ] **Step 7: CHANGELOG** — under `## Unreleased`, first:

```markdown
### Two plugins can hold the same pane still

`neotmux.pane.freeze(id)` hands back a hold, and a pane is frozen while anyone holds it:
`hold:release()` lets go of that one hold and nobody else's, so one plugin can no longer thaw a
pane out from under another. `g.pane.freeze(id)` is the same through a group, and the group
releases it when it is emptied, so a reload never leaves a pane frozen. `neotmux.pane.unfreeze`
is gone. Copy mode and hints take their holds this way and no longer need to ask
`pane.frozen` first.
```

- [ ] **Step 8: Whole-workspace check and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets && cargo test --workspace`
Expected: fmt clean, no clippy warnings, every `test result: ok`. (`neotmux/tests/idle.rs` is
known flaky under the full parallel run — #51; if it alone fails, re-run
`cargo test -p neotmux --test idle` and report both results.)

```bash
git add docs/plugins.md CHANGELOG.md
git commit -m "docs: a freeze is a hold, and a group lets go of its own" # plus attribution lines
```

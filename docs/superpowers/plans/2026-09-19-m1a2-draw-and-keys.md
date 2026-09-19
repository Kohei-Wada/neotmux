# neotmux M1a part 2 — drawing, keys and the fixture plugin set

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A plugin can draw on the screen and consume keys, and the milestone's real exit criterion — a fixture plugin set exercising the whole ABI — passes.

**Architecture:** Plugins draw by calling primitives on a context object whose calls append to a per-plugin command buffer; the host applies the buffer after the tick, so a tick that fails can be discarded and the previous frame's buffer replayed. Each buffer is stamped with an epoch (grid size, live pane ids, focus) and a stale buffer is only replayed when its epoch still matches. Key callbacks return whether they consumed the key, which is what keeps the rest of the bytes flowing to the PTY.

**Tech Stack:** Rust 1.95 edition 2024, `mlua` 0.12 (lua54, vendored), `ratatui` 0.30, the existing four-crate workspace.

**Spec:** `docs/superpowers/specs/2026-09-19-neotmux-design.md` — §4.4, §4.5, §4.8, §4.9's notification surface, and §4.11's exit criteria. §4.1–4.3, §4.6, §4.7 and §4.10 are part 1, already merged. §5 is M1b.

## Global Constraints

- Edition 2024, rustc 1.95 or newer. Dependency versions are declared once in `[workspace.dependencies]`; members reference them with `workspace = true`
- `neotmux-core` and `neotmux-term` stay free of Lua. `neotmux-lua` never touches ratatui or a PTY. **Plugins never see ratatui types** — the command enum lives in `neotmux-core::proto`
- Plugins are fully trusted (spec §8). The threat model is accidents, not malice
- All plugin→host intent goes through the op buffer, drained at one point per frame; no host binding re-enters Lua (§4.6)
- No test may depend on the process's current directory
- Indentation is spaces, never tabs. No code comments unless the *why* is non-obvious, and then one line
- **No `#[allow(...)]` to silence a lint the code can satisfy.** This has been enforced on every branch of this project so far
- The M0a invariant still binds: no code path may leave the terminal in raw mode or in the alternate screen
- `crates/neotmux/tests/idle.rs` asserts nothing is written while nothing happens. A tick that produces no commands must not mark the frame dirty
- Every task ends with `make check` and `cargo test --workspace --release` both green, then a commit

## What already exists

```rust
// neotmux-lua
pub struct LuaSubsystem;
impl LuaSubsystem {
    pub fn build(layers: runtime::Layers) -> anyhow::Result<LuaSubsystem>;
    pub fn run_init(&self) -> Vec<LoadFailure>;
    pub fn dispatch_tick(&self);                      // this task replaces its body
    pub fn drain_ops(&self) -> Vec<(PluginId, Op)>;
    pub fn registry(&self) -> &Registry;
    pub fn plugins(&self) -> &Plugins;
    pub fn opt(&self) -> Opts;
}
pub enum Kind { Tick, Event(String), Keymap(String), Status }
pub enum KindTag { Tick, Event, Keymap, Status }
impl Registry {
    pub fn add(&self, owner: PluginId, kind: Kind, callback: Rc<mlua::RegistryKey>) -> RegistrationId;
    pub fn remove(&self, id: RegistrationId) -> bool;
    pub fn snapshot(&self, tag: KindTag) -> Vec<RegistrationId>;
    pub fn owner(&self, id: RegistrationId) -> Option<PluginId>;
    pub fn kind(&self, id: RegistrationId) -> Option<Kind>;
    pub fn callback(&self, id: RegistrationId) -> Option<Rc<mlua::RegistryKey>>;
    pub fn remove_all_of(&self, owner: PluginId) -> usize;
}
pub enum Op { Quit, Notify { level: Level, text: String }, Log { level: Level, text: String } }

// neotmux-core::proto
pub struct Size { pub rows: u16, pub cols: u16 }
pub struct PaneId(u32);                                // private field, Display as %N
pub enum Color { Default, Idx(u8), Rgb(u8, u8, u8) }
pub struct SnapCell { pub text: String, pub fg: Color, pub bg: Color, pub bold: bool, … }
pub struct GridSnapshot { pub size: Size, pub cursor: (u16, u16), pub hide_cursor: bool, pub cells: Vec<SnapCell> }

// neotmux binary
pub fn draw_grid(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, grid: &GridSnapshot);
pub fn encode_key(key: crossterm::event::KeyEvent) -> Option<Vec<u8>>;
```

`app::run`'s loop, in order: drain the PTY channel (capped), poll input until the frame budget, `lua.dispatch_tick()`, drain ops, draw if dirty, break on EOF.

---

## File Structure

| path | responsibility |
|---|---|
| `crates/neotmux-core/src/proto/draw.rs` | `DrawCmd`, `Rect`, `StatusFragment`, `Epoch` — types only |
| `crates/neotmux-lua/src/draw.rs` | the per-plugin command buffers and their epochs |
| `crates/neotmux-lua/src/api/ctx.rs` | the `ctx` object handed to a tick callback |
| `crates/neotmux-lua/src/api/keymap.rs` | key registration, conflict detection, key dispatch |
| `crates/neotmux-lua/src/api/event.rs` | event dispatch with payloads |
| `crates/neotmux/src/overlay.rs` | applying `DrawCmd`s and the status line to a ratatui frame |
| `crates/neotmux/src/notify.rs` | the host-owned notification surface |
| `crates/neotmux/tests/fixtures/` | the §4.11 plugin set |
| `crates/neotmux/tests/abi.rs` | §4.11's exit criteria as one integration test |

---

### Task 1: Draw command types

**Files:**
- Create: `crates/neotmux-core/src/proto/draw.rs`
- Modify: `crates/neotmux-core/src/proto/mod.rs`

**Interfaces:**
- Consumes: `proto::{Color, PaneId, Size}`
- Produces:
  - `Rect { x: u16, y: u16, w: u16, h: u16 }` with `contains(x, y) -> bool`
  - `DrawCmd` — `Text { pane: Option<PaneId>, x: u16, y: u16, text: String, fg: Color, bg: Color }`, `Rect { pane: Option<PaneId>, rect: Rect, fg: Color, bg: Color }`, `Line { pane: Option<PaneId>, from: (u16, u16), to: (u16, u16), fg: Color }`, `Marker { pane: Option<PaneId>, x: u16, y: u16, text: String, fg: Color }`
  - `StatusFragment { text: String, fg: Color, bg: Color }`
  - `Epoch { size: Size, panes: Vec<PaneId>, focus: Option<PaneId> }` with `matches(&self, other: &Epoch) -> bool`

`pane: None` means the surface rather than a specific pane — that is what a status provider draws into. Everything is `#[non_exhaustive]`, like the rest of `proto`, because §2.1 promotes this module to a published crate at M3.

`Epoch::matches` compares all three fields; `panes` is kept sorted on construction so the comparison is order-independent.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux-core/src/proto/draw.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{PaneId, Size};

    #[test]
    fn a_rect_knows_what_it_contains() {
        let r = Rect { x: 2, y: 3, w: 4, h: 2 };
        assert!(r.contains(2, 3));
        assert!(r.contains(5, 4));
        assert!(!r.contains(6, 4));
        assert!(!r.contains(5, 5));
        assert!(!r.contains(1, 3));
    }

    #[test]
    fn a_zero_sized_rect_contains_nothing() {
        let r = Rect { x: 0, y: 0, w: 0, h: 0 };
        assert!(!r.contains(0, 0));
    }

    #[test]
    fn epochs_match_when_every_field_matches() {
        let a = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1)], Some(PaneId::from(1)));
        let b = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1)], Some(PaneId::from(1)));
        assert!(a.matches(&b));
    }

    #[test]
    fn a_resize_breaks_the_epoch() {
        let a = Epoch::new(Size { rows: 24, cols: 80 }, vec![], None);
        let b = Epoch::new(Size { rows: 25, cols: 80 }, vec![], None);
        assert!(!a.matches(&b));
    }

    #[test]
    fn a_closed_pane_breaks_the_epoch() {
        let a = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1), PaneId::from(2)], None);
        let b = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1)], None);
        assert!(!a.matches(&b));
    }

    #[test]
    fn a_focus_change_breaks_the_epoch() {
        let a = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1)], Some(PaneId::from(1)));
        let b = Epoch::new(Size { rows: 24, cols: 80 }, vec![PaneId::from(1)], None);
        assert!(!a.matches(&b));
    }

    #[test]
    fn pane_order_does_not_affect_matching() {
        let a = Epoch::new(Size { rows: 1, cols: 1 }, vec![PaneId::from(2), PaneId::from(1)], None);
        let b = Epoch::new(Size { rows: 1, cols: 1 }, vec![PaneId::from(1), PaneId::from(2)], None);
        assert!(a.matches(&b));
    }
}
```

`PaneId::from(u32)` must exist for these tests; if part 1 left only a private constructor, add the `From<u32>` impl in `ids.rs` as part of this task and say so in the report.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-core draw`
Expected: `Rect`/`Epoch` not found.

- [ ] **Step 3: Write the implementation**

`crates/neotmux-core/src/proto/draw.rs`, above the tests:

```rust
use crate::proto::{Color, PaneId, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrawCmd {
    Text {
        pane: Option<PaneId>,
        x: u16,
        y: u16,
        text: String,
        fg: Color,
        bg: Color,
    },
    Rect {
        pane: Option<PaneId>,
        rect: Rect,
        fg: Color,
        bg: Color,
    },
    Line {
        pane: Option<PaneId>,
        from: (u16, u16),
        to: (u16, u16),
        fg: Color,
    },
    Marker {
        pane: Option<PaneId>,
        x: u16,
        y: u16,
        text: String,
        fg: Color,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct StatusFragment {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Epoch {
    size: Size,
    panes: Vec<PaneId>,
    focus: Option<PaneId>,
}

impl Epoch {
    pub fn new(size: Size, mut panes: Vec<PaneId>, focus: Option<PaneId>) -> Self {
        panes.sort();
        Self { size, panes, focus }
    }

    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}
```

`Epoch`'s fields are private so the sorted invariant cannot be broken from outside; `new` is the only constructor.

`crates/neotmux-core/src/proto/mod.rs`: add `pub mod draw;` and re-export `draw::{DrawCmd, Epoch, Rect, StatusFragment}`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-core`
Expected: the existing tests plus 7 new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-core
git commit -m "feat(core): draw commands, status fragments and frame epochs"
```

---

### Task 2: Per-plugin command buffers

**Files:**
- Create: `crates/neotmux-lua/src/draw.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`

**Interfaces:**
- Consumes: `neotmux_core::proto::{DrawCmd, Epoch, StatusFragment}`, `plugin::PluginId`
- Produces:
  - `draw::Buffers` — `new()`, `begin(&self, epoch: Epoch)`, `record(&self, owner: PluginId, cmd: DrawCmd)`, `status(&self, owner: PluginId, fragment: StatusFragment)`, `commit(&self, owner: PluginId)`, `discard(&self, owner: PluginId)`, `applicable(&self, now: &Epoch) -> (Vec<DrawCmd>, Vec<StatusFragment>)`, `forget(&self, owner: PluginId)`
  - `draw::Committed { epoch: Epoch, cmds: Vec<DrawCmd>, status: Vec<StatusFragment> }`

This is the mechanism §4.5 exists for. A tick records into a scratch area; `commit` moves the scratch into that plugin's last-known-good buffer, stamped with the epoch that was current when `begin` was called; `discard` throws the scratch away and leaves the previous buffer intact. `applicable` returns everything from every plugin whose stored epoch still matches the current one, which is what the host draws.

Why the scratch/commit split rather than recording straight into the buffer: a tick that raises must leave the plugin's previous frame untouched, and M1b will discard a tick that overruns its budget the same way. Recording in place would corrupt the fallback.

`forget` drops a plugin's buffer entirely, for when its registrations are withdrawn.

Ordering matters for what lands on screen: `applicable` returns plugins in `PluginId` order, which is interning order, which is load order. Say so in the report — it is the only determinism a plugin author gets and it will become a question.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux-lua/src/draw.rs`:

```rust
#[cfg(test)]
mod tests {
    use neotmux_core::proto::{Color, DrawCmd, Epoch, Size, StatusFragment};

    use super::*;
    use crate::plugin::Plugins;

    fn epoch(rows: u16) -> Epoch {
        Epoch::new(Size { rows, cols: 80 }, vec![], None)
    }

    fn text(s: &str) -> DrawCmd {
        DrawCmd::Text {
            pane: None,
            x: 0,
            y: 0,
            text: s.to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }
    }

    fn fragment(s: &str) -> StatusFragment {
        StatusFragment {
            text: s.to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }
    }

    #[test]
    fn a_committed_buffer_is_applicable_under_the_same_epoch() {
        let plugins = Plugins::new();
        let a = plugins.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.status(a, fragment("st"));
        buffers.commit(a);
        let (cmds, status) = buffers.applicable(&epoch(24));
        assert_eq!(cmds, vec![text("hi")]);
        assert_eq!(status, vec![fragment("st")]);
    }

    #[test]
    fn a_stale_buffer_is_not_applicable() {
        let plugins = Plugins::new();
        let a = plugins.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.commit(a);
        let (cmds, status) = buffers.applicable(&epoch(25));
        assert!(cmds.is_empty());
        assert!(status.is_empty());
    }

    #[test]
    fn discard_leaves_the_previous_buffer_intact() {
        let plugins = Plugins::new();
        let a = plugins.intern("a");
        let buffers = Buffers::new();

        buffers.begin(epoch(24));
        buffers.record(a, text("good"));
        buffers.commit(a);

        buffers.begin(epoch(24));
        buffers.record(a, text("half-written"));
        buffers.discard(a);

        let (cmds, _) = buffers.applicable(&epoch(24));
        assert_eq!(cmds, vec![text("good")]);
    }

    #[test]
    fn a_commit_replaces_rather_than_appends() {
        let plugins = Plugins::new();
        let a = plugins.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("first"));
        buffers.commit(a);
        buffers.begin(epoch(24));
        buffers.record(a, text("second"));
        buffers.commit(a);
        let (cmds, _) = buffers.applicable(&epoch(24));
        assert_eq!(cmds, vec![text("second")]);
    }

    #[test]
    fn nothing_recorded_means_nothing_applicable() {
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        let (cmds, status) = buffers.applicable(&epoch(24));
        assert!(cmds.is_empty());
        assert!(status.is_empty());
    }

    #[test]
    fn plugins_are_applied_in_id_order() {
        let plugins = Plugins::new();
        let a = plugins.intern("a");
        let b = plugins.intern("b");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(b, text("second"));
        buffers.record(a, text("first"));
        buffers.commit(b);
        buffers.commit(a);
        let (cmds, _) = buffers.applicable(&epoch(24));
        assert_eq!(cmds, vec![text("first"), text("second")]);
    }

    #[test]
    fn forget_drops_a_plugins_buffer() {
        let plugins = Plugins::new();
        let a = plugins.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("hi"));
        buffers.commit(a);
        buffers.forget(a);
        let (cmds, _) = buffers.applicable(&epoch(24));
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_scratch_left_uncommitted_does_not_leak_into_the_next_tick() {
        let plugins = Plugins::new();
        let a = plugins.intern("a");
        let buffers = Buffers::new();
        buffers.begin(epoch(24));
        buffers.record(a, text("orphan"));
        buffers.begin(epoch(24));
        buffers.record(a, text("fresh"));
        buffers.commit(a);
        let (cmds, _) = buffers.applicable(&epoch(24));
        assert_eq!(cmds, vec![text("fresh")]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua draw`
Expected: `Buffers` not found.

- [ ] **Step 3: Write the implementation**

`crates/neotmux-lua/src/draw.rs`, above the tests:

```rust
use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_core::proto::{DrawCmd, Epoch, StatusFragment};

use crate::plugin::PluginId;

#[derive(Debug, Clone, Default)]
pub struct Committed {
    pub epoch: Epoch,
    pub cmds: Vec<DrawCmd>,
    pub status: Vec<StatusFragment>,
}

#[derive(Default)]
struct Inner {
    epoch: Epoch,
    scratch: BTreeMap<PluginId, Committed>,
    committed: BTreeMap<PluginId, Committed>,
}

#[derive(Default)]
pub struct Buffers {
    inner: RefCell<Inner>,
}

impl Buffers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&self, epoch: Epoch) {
        let mut inner = self.inner.borrow_mut();
        inner.epoch = epoch;
        inner.scratch.clear();
    }

    pub fn record(&self, owner: PluginId, cmd: DrawCmd) {
        let mut inner = self.inner.borrow_mut();
        let epoch = inner.epoch.clone();
        inner
            .scratch
            .entry(owner)
            .or_insert_with(|| Committed { epoch, ..Committed::default() })
            .cmds
            .push(cmd);
    }

    pub fn status(&self, owner: PluginId, fragment: StatusFragment) {
        let mut inner = self.inner.borrow_mut();
        let epoch = inner.epoch.clone();
        inner
            .scratch
            .entry(owner)
            .or_insert_with(|| Committed { epoch, ..Committed::default() })
            .status
            .push(fragment);
    }

    pub fn commit(&self, owner: PluginId) {
        let mut inner = self.inner.borrow_mut();
        if let Some(entry) = inner.scratch.remove(&owner) {
            inner.committed.insert(owner, entry);
        }
    }

    pub fn discard(&self, owner: PluginId) {
        self.inner.borrow_mut().scratch.remove(&owner);
    }

    pub fn forget(&self, owner: PluginId) {
        let mut inner = self.inner.borrow_mut();
        inner.scratch.remove(&owner);
        inner.committed.remove(&owner);
    }

    pub fn applicable(&self, now: &Epoch) -> (Vec<DrawCmd>, Vec<StatusFragment>) {
        let inner = self.inner.borrow();
        let mut cmds = Vec::new();
        let mut status = Vec::new();
        for entry in inner.committed.values() {
            if !entry.epoch.matches(now) {
                continue;
            }
            cmds.extend(entry.cmds.iter().cloned());
            status.extend(entry.status.iter().cloned());
        }
        (cmds, status)
    }
}
```

`begin` clearing the scratch is what makes `a_scratch_left_uncommitted_does_not_leak_into_the_next_tick` pass: a tick that raised before committing leaves its half-written commands behind, and the next `begin` throws them away.

`crates/neotmux-lua/src/lib.rs`: add `pub mod draw;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-lua`
Expected: everything from part 1 plus 8 new tests.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-lua
git commit -m "feat(lua): per-plugin draw buffers with epoch-guarded replay"
```

---

### Task 3: The `ctx` object and tick dispatch

**Files:**
- Create: `crates/neotmux-lua/src/api/ctx.rs`
- Modify: `crates/neotmux-lua/src/api/mod.rs`, `crates/neotmux-lua/src/lib.rs`
- Test: `crates/neotmux-lua/tests/drawing.rs`

**Interfaces:**
- Consumes: `draw::Buffers`, `plugin::{PluginId, Plugins}`, `proto::{Color, DrawCmd, PaneId, Rect, Size, StatusFragment}`
- Produces:
  - `ctx::install(lua, buffers, geometry) -> mlua::Result<()>` — registers the userdata type
  - `LuaSubsystem::begin_frame(&self, epoch: Epoch, size: Size, panes: Vec<(PaneId, Rect)>, focus: Option<PaneId>)`
  - `LuaSubsystem::dispatch_tick(&self)` — now passes a `ctx` and commits or discards per plugin
  - `LuaSubsystem::frame_output(&self) -> (Vec<DrawCmd>, Vec<StatusFragment>)`

The Lua surface, from §4.5:

```lua
neotmux.on_tick(function(ctx)
  local w, h = ctx:size()
  local rect = ctx:pane_rect(pane_id)        -- nil for an unknown pane
  ctx:text(pane_id, x, y, "hello")           -- pane_id may be nil for the whole surface
  ctx:rect(pane_id, x, y, w, h)
  ctx:line(pane_id, x1, y1, x2, y2)
  ctx:marker(pane_id, x, y, "*")
  ctx:status("12:34")                        -- a status fragment
end)
```

Colours are optional trailing arguments on each primitive, defaulting to `Color::Default`; accept them as an optional table `{ fg = …, bg = … }` where a colour is either an integer (palette index) or a string `"#rrggbb"`. Reject anything else with a Lua error naming the argument — a plugin passing a bad colour should find out at the call, not by seeing nothing drawn.

`ctx` is an `mlua` `UserData` created per dispatch inside `Lua::scope`, so a plugin cannot stash it and draw outside its tick. That is the point: the buffer's epoch is only valid for the frame it was begun in. A stashed `ctx` used later must raise rather than silently record into the wrong frame — test it.

`dispatch_tick` becomes: for each tick registration in snapshot order, push the owner, call with the `ctx`, pop, then `commit` on `Ok` and `discard` on `Err`. An error is logged through the op buffer with the plugin's name, which is what part 1 deliberately left undone.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/drawing.rs`:

```rust
use std::fs;

use neotmux_core::proto::{DrawCmd, Epoch, PaneId, Rect, Size};
use neotmux_lua::runtime::Layers;
use neotmux_lua::LuaSubsystem;

fn layer(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    dir
}

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = layer(files);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn frame(sub: &LuaSubsystem, rows: u16) {
    let size = Size { rows, cols: 80 };
    let pane = PaneId::from(1);
    let rect = Rect { x: 0, y: 0, w: 80, h: rows };
    sub.begin_frame(
        Epoch::new(size, vec![pane], Some(pane)),
        size,
        vec![(pane, rect)],
        Some(pane),
    );
    sub.dispatch_tick();
}

#[test]
fn a_tick_records_what_it_draws() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 1, 2, 'hi') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        DrawCmd::Text { x, y, text, .. } => {
            assert_eq!((*x, *y), (1, 2));
            assert_eq!(text, "hi");
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn ctx_size_reports_the_surface() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.sizer')"),
        (
            "lua/plugin/sizer.lua",
            "neotmux.on_tick(function(ctx) local w, h = ctx:size() ctx:text(nil, 0, 0, w .. 'x' .. h) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    match &cmds[0] {
        DrawCmd::Text { text, .. } => assert_eq!(text, "80x24"),
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn pane_rect_answers_for_a_live_pane_and_nil_otherwise() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.rects')"),
        (
            "lua/plugin/rects.lua",
            r#"
            neotmux.on_tick(function(ctx)
              local r = ctx:pane_rect(1)
              ctx:text(nil, 0, 0, r and (r.w .. 'x' .. r.h) or 'nil')
              ctx:text(nil, 0, 1, ctx:pane_rect(99) and 'some' or 'nil')
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    let texts: Vec<String> = cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["80x24".to_string(), "nil".to_string()]);
}

#[test]
fn a_status_fragment_reaches_the_host() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.clock')"),
        (
            "lua/plugin/clock.lua",
            "neotmux.on_tick(function(ctx) ctx:status('12:34') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let (_, status) = sub.frame_output();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].text, "12:34");
}

#[test]
fn a_tick_that_raises_keeps_its_previous_frame() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.flaky')"),
        (
            "lua/plugin/flaky.lua",
            r#"
            local n = 0
            neotmux.on_tick(function(ctx)
              n = n + 1
              ctx:text(nil, 0, 0, 'frame' .. n)
              if n == 2 then error('boom') end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    let (first, _) = sub.frame_output();
    frame(&sub, 24);
    let (second, _) = sub.frame_output();
    assert_eq!(first, second, "the failed tick replaced the good frame");
}

#[test]
fn a_tick_error_is_reported_with_the_plugin_name() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.flaky')"),
        (
            "lua/plugin/flaky.lua",
            "neotmux.on_tick(function() error('boom') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 1, "expected one op, got {ops:?}");
    assert_eq!(sub.plugins().name(ops[0].0).as_deref(), Some("flaky"));
}

#[test]
fn a_resize_makes_the_previous_frame_inapplicable() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            r#"
            local drawn = false
            neotmux.on_tick(function(ctx)
              if not drawn then ctx:text(nil, 0, 0, 'once') drawn = true end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(sub.frame_output().0.len(), 1);
    frame(&sub, 25);
    assert!(sub.frame_output().0.is_empty(), "a stale frame was replayed after a resize");
}

#[test]
fn a_stashed_ctx_cannot_draw_outside_its_tick() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.thief')"),
        (
            "lua/plugin/thief.lua",
            r#"
            local stolen
            neotmux.on_tick(function(ctx)
              if stolen then
                local ok = pcall(function() stolen:text(nil, 0, 0, 'late') end)
                ctx:text(nil, 0, 0, ok and 'allowed' or 'refused')
              else
                stolen = ctx
              end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    match &cmds[0] {
        DrawCmd::Text { text, .. } => assert_eq!(text, "refused"),
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn a_bad_colour_raises_at_the_call() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'x', { fg = 'not a colour' }) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 1, "a bad colour should have raised");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua --test drawing`
Expected: `begin_frame` / `frame_output` not found.

- [ ] **Step 3: Write the implementation**

Build `ctx.rs` around a `Ctx` struct holding `PluginId`, `Rc<Buffers>`, the surface `Size`, the pane rects and the focus, exposed to Lua with `UserData`. Create it inside `Lua::scope` in `dispatch_tick` so the borrow ends with the call and a stashed reference is dead — mlua's scoped userdata raises `CallbackDestructed` when used after the scope closes, which is what the stash test asserts.

`begin_frame` stores the geometry and calls `Buffers::begin`. `frame_output` calls `Buffers::applicable` with the current epoch.

Colour parsing lives in one helper used by every primitive.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `make check`
Expected: green, including every part 1 test.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-lua
git commit -m "feat(lua): a scoped draw context, committed per tick"
```

---

### Task 4: Key consumption, prefix ownership and conflicts

**Files:**
- Create: `crates/neotmux-lua/src/api/keymap.rs`
- Modify: `crates/neotmux-lua/src/api/mod.rs`, `crates/neotmux-lua/src/lib.rs`
- Test: `crates/neotmux-lua/tests/keys.rs`

**Interfaces:**
- Consumes: `registry::{Kind, KindTag, Registry}`, `plugin::Plugins`, `ops::OpBuffer`
- Produces:
  - `keymap::normalise(spec: &str) -> Option<String>` — the canonical form of a key spec
  - `LuaSubsystem::dispatch_key(&self, spec: &str) -> KeyOutcome`
  - `pub enum KeyOutcome { Consumed, Fallthrough }`
  - `neotmux.keymap.set(spec, fn)` returns a handle, or `nil, message` on conflict

§4.4, and every bullet of it is ABI:

- **the callback's return value decides**: `true` consumes the key, `false` or `nil` lets it through to the PTY. Introducing this later would break every plugin written against M1a, which is why it is fixed now even though nothing in this milestone has a second consumer
- **the prefix key is owned by the host** and is never delivered to a plugin or the PTY. `Opts::prefix` holds it, defaulting to `C-b`
- **two plugins binding the same key is an error at registration time.** The second `keymap.set` returns `nil` plus a message and logs the conflict with both plugin names. Last-wins is rejected because it makes load order load-bearing and silent

`normalise` exists so that `C-b`, `c-b` and `Ctrl-b` are the same binding for conflict purposes. Define the canonical form as lowercase with `C-`, `M-` and `S-` prefixes in that order, and reject anything unparseable at registration with a message naming the spec — a plugin binding `Ctrl+Shift+F13` should learn immediately.

A callback that raises is treated as fallthrough, and the error is reported through the op buffer. Refusing to pass a key to the PTY because a plugin is broken would make a broken plugin able to lock the user out of their shell.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/keys.rs`:

```rust
use std::fs;

use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    (dir, sub)
}

#[test]
fn returning_true_consumes_the_key() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.binder')"),
        (
            "lua/plugin/binder.lua",
            "neotmux.keymap.set('C-x', function() return true end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.dispatch_key("C-x"), KeyOutcome::Consumed);
}

#[test]
fn returning_false_or_nil_falls_through() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.binder')"),
        (
            "lua/plugin/binder.lua",
            r#"
            neotmux.keymap.set('C-x', function() return false end)
            neotmux.keymap.set('C-y', function() end)
            return {}
            "#,
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.dispatch_key("C-x"), KeyOutcome::Fallthrough);
    assert_eq!(sub.dispatch_key("C-y"), KeyOutcome::Fallthrough);
}

#[test]
fn an_unbound_key_falls_through() {
    let (_d, sub) = built(&[("init.lua", "")]);
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.dispatch_key("C-z"), KeyOutcome::Fallthrough);
}

#[test]
fn key_specs_are_normalised_for_conflict_purposes() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", "neotmux.keymap.set('C-x', function() return true end) return {}"),
        (
            "lua/plugin/b.lua",
            "local h, err = neotmux.keymap.set('c-X', function() return true end) if h ~= nil then error('conflict not detected') end return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty(), "the conflicting registration should not raise");
    assert_eq!(sub.dispatch_key("C-x"), KeyOutcome::Consumed);
}

#[test]
fn a_conflict_returns_nil_and_a_message_and_logs_both_names() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.first') require('plugin.second')"),
        ("lua/plugin/first.lua", "neotmux.keymap.set('C-x', function() return true end) return {}"),
        (
            "lua/plugin/second.lua",
            r#"
            local h, err = neotmux.keymap.set('C-x', function() return true end)
            neotmux.log.warn('handle=' .. tostring(h) .. ' err=' .. tostring(err))
            return {}
            "#,
        ),
    ]);
    assert!(sub.run_init().is_empty());
    let ops = sub.drain_ops();
    let text = format!("{ops:?}");
    assert!(text.contains("handle=nil"), "{text}");
    assert!(text.contains("first"), "the conflict message should name the holder: {text}");
}

#[test]
fn the_loser_of_a_conflict_does_not_shadow_the_winner() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.first') require('plugin.second')"),
        ("lua/plugin/first.lua", "neotmux.keymap.set('C-x', function() return true end) return {}"),
        ("lua/plugin/second.lua", "neotmux.keymap.set('C-x', function() return false end) return {}"),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.dispatch_key("C-x"), KeyOutcome::Consumed);
}

#[test]
fn an_unparseable_spec_is_refused_at_registration() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.bad')"),
        (
            "lua/plugin/bad.lua",
            "local h, err = neotmux.keymap.set('Ctrl+Shift+F13', function() end) if h ~= nil then error('bad spec accepted') end return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
}

#[test]
fn a_raising_callback_falls_through_rather_than_swallowing_the_key() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.flaky')"),
        ("lua/plugin/flaky.lua", "neotmux.keymap.set('C-x', function() error('boom') end) return {}"),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.dispatch_key("C-x"), KeyOutcome::Fallthrough);
    assert_eq!(sub.drain_ops().len(), 1, "the error should be reported");
}

#[test]
fn removing_a_binding_frees_the_key_for_another_plugin() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.transient') require('plugin.later')"),
        (
            "lua/plugin/transient.lua",
            "local h = neotmux.keymap.set('C-x', function() return true end) h:remove() return {}",
        ),
        (
            "lua/plugin/later.lua",
            "local h = neotmux.keymap.set('C-x', function() return true end) if h == nil then error('key was not freed') end return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.dispatch_key("C-x"), KeyOutcome::Consumed);
}

#[test]
fn normalise_is_canonical_and_total() {
    use neotmux_lua::keymap::normalise;
    assert_eq!(normalise("C-x").as_deref(), Some("C-x"));
    assert_eq!(normalise("c-X").as_deref(), Some("C-x"));
    assert_eq!(normalise("Ctrl-x").as_deref(), Some("C-x"));
    assert_eq!(normalise("M-C-x").as_deref(), Some("C-M-x"));
    assert_eq!(normalise("Ctrl+Shift+F13"), None);
    assert_eq!(normalise(""), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua --test keys`
Expected: `KeyOutcome` / `dispatch_key` not found.

- [ ] **Step 3: Write the implementation**

`normalise` parses the modifier prefixes in any order and any case, emits them in `C-`, `M-`, `S-` order, lowercases a single-character key and title-cases a named one, and returns `None` for anything it cannot parse.

Conflict detection reads the existing `Kind::Keymap(spec)` registrations through `Registry::snapshot(KindTag::Keymap)` + `Registry::kind`, comparing normalised specs. The conflict message names the holding plugin, found through `Registry::owner` and `Plugins::name`.

`dispatch_key` normalises, finds the one matching registration, pushes the owner, calls, pops, and maps the Lua return to `KeyOutcome`. `Err` is `Fallthrough` plus an op.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `make check`

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-lua
git commit -m "feat(lua): key bindings with a consumption contract and conflict detection"
```

---

### Task 5: Events with payloads, and resize ordering

**Files:**
- Create: `crates/neotmux-lua/src/api/event.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`, `crates/neotmux-lua/src/api/mod.rs`
- Test: `crates/neotmux-lua/tests/events.rs`

**Interfaces:**
- Produces:
  - `pub enum Event { PaneOpened { pane: PaneId }, PaneClosed { pane: PaneId }, PaneOutput { pane: PaneId, bytes: Vec<u8> }, FocusChanged { pane: Option<PaneId> }, Resized { size: Size } }`
  - `LuaSubsystem::dispatch_event(&self, event: &Event)`

Each event becomes a Lua table passed to every subscriber registered for that name. `pane_output` carries the bytes as a Lua string — `watcher` in §4.11 exists to prove a plugin can observe what a pane produced, and it is the only event whose payload is large, so it is also the one where copying per subscriber would hurt. Pass the same `mlua::String` to every subscriber rather than building one each.

Subscribers are dispatched in registration order; one that raises does not stop the others, and its error is reported through the op buffer with its plugin's name.

Spec §4.8 fixes the resize order, and this task is where it becomes real in the Lua-facing sense:

1. terminal resize observed
2. `vt100::Screen::set_size` and PTY `resize()` applied
3. pane rects recomputed
4. `resized` event dispatched
5. tick dispatched — `ctx:size()` already returns the new values
6. render

So `dispatch_event(&Event::Resized { … })` must be called by the host **after** it has updated the geometry and **before** `begin_frame`/`dispatch_tick` for that frame. Task 6 wires that; this task provides the mechanism and a test that a `resized` subscriber sees the new size.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/events.rs`:

```rust
use std::fs;

use neotmux_core::proto::{PaneId, Size};
use neotmux_lua::runtime::Layers;
use neotmux_lua::{Event, LuaSubsystem};

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    (dir, sub)
}

#[test]
fn a_subscriber_sees_the_payload_of_pane_output() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        (
            "lua/plugin/watcher.lua",
            "neotmux.on_event('pane_output', function(e) neotmux.log.info('saw ' .. e.bytes) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(&Event::PaneOutput {
        pane: PaneId::from(1),
        bytes: b"hello".to_vec(),
    });
    let ops = sub.drain_ops();
    assert!(format!("{ops:?}").contains("saw hello"), "{ops:?}");
}

#[test]
fn resized_carries_the_new_size() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.sizer')"),
        (
            "lua/plugin/sizer.lua",
            "neotmux.on_event('resized', function(e) neotmux.log.info(e.rows .. 'x' .. e.cols) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(&Event::Resized {
        size: Size { rows: 30, cols: 100 },
    });
    assert!(format!("{:?}", sub.drain_ops()).contains("30x100"));
}

#[test]
fn every_subscriber_runs_even_when_one_raises() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.bad') require('plugin.good')"),
        (
            "lua/plugin/bad.lua",
            "neotmux.on_event('pane_closed', function() error('boom') end) return {}",
        ),
        (
            "lua/plugin/good.lua",
            "neotmux.on_event('pane_closed', function() neotmux.log.info('still here') end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(&Event::PaneClosed { pane: PaneId::from(1) });
    let text = format!("{:?}", sub.drain_ops());
    assert!(text.contains("still here"), "{text}");
    assert!(text.contains("boom"), "the raising subscriber's error should be reported: {text}");
}

#[test]
fn a_subscriber_to_another_event_is_not_called() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.picky')"),
        (
            "lua/plugin/picky.lua",
            "neotmux.on_event('pane_opened', function() neotmux.log.info('opened') end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(&Event::PaneClosed { pane: PaneId::from(1) });
    assert!(sub.drain_ops().is_empty());
}

#[test]
fn focus_changed_can_carry_no_pane() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.focus')"),
        (
            "lua/plugin/focus.lua",
            "neotmux.on_event('focus_changed', function(e) neotmux.log.info('focus ' .. tostring(e.pane)) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(&Event::FocusChanged { pane: None });
    assert!(format!("{:?}", sub.drain_ops()).contains("focus nil"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua --test events`

- [ ] **Step 3: Write the implementation**

One `Event` enum, one `dispatch_event` that builds the payload table once and calls every subscriber for that event name in registration order, with the same push/pop identity discipline as `dispatch_tick`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `make check`

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-lua
git commit -m "feat(lua): events with payloads, dispatched in registration order"
```

---

### Task 6: Applying it all in the binary

**Files:**
- Create: `crates/neotmux/src/overlay.rs`, `crates/neotmux/src/notify.rs`
- Modify: `crates/neotmux/src/app.rs`, `crates/neotmux/src/main.rs`, `crates/neotmux/src/render.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–5
- Produces:
  - `overlay::apply(frame: &mut Frame, area: Rect, cmds: &[DrawCmd], panes: &PaneRects)`
  - `overlay::draw_status(frame: &mut Frame, area: Rect, fragments: &[StatusFragment])`
  - `notify::Notifications` — a host-owned surface: `push(&mut self, level, tag, text)`, `render(&self, frame, area)`, `expire(&mut self, now: Instant)`

This is where the milestone becomes visible. The loop's order, per §4.8 and §4.6:

1. drain the PTY channel (capped), feeding `TermState`
2. poll input; for each key, **normalise it and ask `dispatch_key`** — `Consumed` stops there, `Fallthrough` encodes and writes to the PTY. The prefix key is swallowed by the host and never reaches either
3. on a resize: update `TermState` and the PTY, recompute pane rects, then `dispatch_event(Resized)`
4. `begin_frame(epoch, size, panes, focus)` then `dispatch_tick()`
5. drain ops once, at one place: `Quit` returns, `Log` logs, **`Notify` now goes to the notification surface as well as the log**
6. if dirty, draw: the grid, then `overlay::apply`, then the status line, then notifications

**The status line costs a row.** Reserve the bottom row for it only when at least one status fragment exists, and give the pane the rest; otherwise the pane gets the whole area, exactly as today. That means the pane's size can change when a status provider appears or disappears, so the PTY must be resized then too — treat it like any other geometry change and recompute the epoch.

**`idle.rs` must keep passing.** A tick that records nothing and produces no status must not set `dirty`. Set it from "the applicable output differs from what was drawn last frame", not from "a tick ran". The cheapest honest version is to keep the previous `(Vec<DrawCmd>, Vec<StatusFragment>)` and compare; both derive `PartialEq` for exactly this.

Notifications expire after a few seconds and are drawn over the top-right of the surface. They are host-owned because §4.9 says so and because a notification about the plugin system failing cannot be rendered by a plugin.

- [ ] **Step 1: Write the failing test**

Add to `crates/neotmux/tests/abi.rs` (created here, extended in Task 7):

```rust
use std::fs;
use std::time::Duration;

mod harness;
use harness::Neotmux;

#[test]
fn a_status_provider_reaches_the_screen() {
    let dir = harness::runtime(&[
        ("init.lua", "require('plugin.clock')"),
        (
            "lua/plugin/clock.lua",
            "neotmux.on_tick(function(ctx) ctx:status('STATUS-MARKER') end) return {}",
        ),
    ]);
    let mut nt = Neotmux::start(&dir);
    nt.wait_for_screen("STATUS-MARKER", Duration::from_secs(5));
}

#[test]
fn a_plugin_can_draw_over_the_pane() {
    let dir = harness::runtime(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'OVERLAY') end) return {}",
        ),
    ]);
    let mut nt = Neotmux::start(&dir);
    nt.wait_for_screen("OVERLAY", Duration::from_secs(5));
}

#[test]
fn a_consumed_key_never_reaches_the_shell() {
    let dir = harness::runtime(&[
        ("init.lua", "require('plugin.binder')"),
        (
            "lua/plugin/binder.lua",
            r#"
            neotmux.keymap.set('C-x', function() return true end)
            neotmux.keymap.set('C-y', function() return false end)
            return {}
            "#,
        ),
    ]);
    let mut nt = Neotmux::start_with_shell(&dir, "cat");
    nt.send("\x18");              // C-x, consumed
    nt.send("\x19");              // C-y, falls through
    nt.send("\r");
    let screen = nt.screen_after(Duration::from_secs(2));
    assert!(!screen.contains("^X"), "the consumed key reached the shell: {screen:?}");
    assert!(screen.contains("^Y"), "the fallthrough key did not reach the shell: {screen:?}");
}

#[test]
fn nothing_is_written_while_nothing_happens() {
    let dir = harness::runtime(&[("init.lua", "")]);
    let mut nt = Neotmux::start(&dir);
    nt.settle(Duration::from_secs(1));
    assert_eq!(nt.bytes_in(Duration::from_secs(1)), 0);
}
```

`crates/neotmux/tests/harness/mod.rs` is a shared PTY driver: it starts the binary with `HOME`, `XDG_CONFIG_HOME` and `--log` pointed into a tempdir (the isolation the previous milestone added), feeds the master, and renders what neotmux writes so a test can assert on the *screen* rather than on escape bytes. `terminal_restore.rs` already has most of this; lift the common parts rather than writing a third copy, and leave the existing tests working.

Rendering the screen in Rust means parsing what neotmux emits. `vt100` is already a dependency of `neotmux-term`; add it as a dev-dependency of `neotmux` and feed the captured bytes to a `vt100::Parser`, then read `screen().contents()`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux --test abi`
Expected: nothing draws; the markers never appear.

- [ ] **Step 3: Write the implementation**

Wire the loop as described. `overlay::apply` clips every command to its pane's rect (or to the surface for `pane: None`) and ignores a command naming a pane that no longer exists — the epoch already prevents most of that, but a command for a pane closed *during* the same frame is still possible.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `make check` and `cargo test --workspace --release`
Expected: green, including `idle.rs`, `terminal_restore.rs`, `pty_flood.rs` and `plugin_smoke.rs`.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux
git commit -m "feat: draw plugin overlays, the status line and notifications"
```

---

### Task 7: The fixture plugin set

**Files:**
- Create: `crates/neotmux/tests/fixtures/*.lua`
- Modify: `crates/neotmux/tests/abi.rs`

**Interfaces:**
- Consumes: the whole ABI

Spec §4.11 is the milestone's real exit criterion, and it is a plugin *set* rather than one plugin because the point is to exercise the ABI as a whole:

- `clock` — a status provider using `ctx:size()`, asserted to produce a non-empty command buffer every frame
- `watcher` — consumes a payload-carrying `pane_output` event, asserted to observe bytes a harness wrote to the PTY
- `binder` — `keymap.set` returning `true` for one key and `false` for another, asserted that the consumed key never reaches the PTY and the other does
- `conflicting` — binds the same key as `binder`, asserted that registration returns `nil`, both names appear in the log, and `binder`'s binding still works
- `configured` — reads a config holder that `init.lua` mutated, asserted to see the mutated value
- `remover` — registers then `:remove()`s twice, asserted idempotent and that no callback runs afterwards
- a plugin calling the M2 pane/split API against a stub, to freeze those signatures now

Plus: every callback's attributed `PluginId` matches the plugin that registered it, including one registered from inside another plugin's dispatch.

The last bullet needs a decision this plan makes explicitly: **M2's pane API does not exist yet, so the stub is `neotmux.pane` with `list()`, `current()`, `split(direction)` and `focus(pane_id)`**, where `split` and `focus` push ops the host currently logs and ignores. Freezing the signatures now is the whole point — a plugin written against them keeps working when M2 implements them.

Write the fixtures as real files under `tests/fixtures/` and have the harness copy them into the runtime layer, rather than embedding Lua in Rust string literals. They are the closest thing this project has to example plugins and someone will read them as documentation.

- [ ] **Step 1: Write the fixtures and the failing test**

Each fixture is a small file; the test asserts the bullets above against a single running instance with all of them loaded. Assert attribution by having each fixture log a line and checking the tags.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p neotmux --test abi`
Expected: `neotmux.pane` does not exist.

- [ ] **Step 3: Implement the pane stub and make the set pass**

- [ ] **Step 4: Run the whole suite**

Run: `make check` and `cargo test --workspace --release`

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/tests
git commit -m "test: the M1a exit-criteria plugin set"
```

---

### Task 8: Documentation

**Files:**
- Create: `docs/plugins.md`
- Modify: `README.md`

Nothing in this repository yet tells a plugin author what the ABI is. The fixture set is example code; this is the reference that names every call, says what a tick may and may not do, and states the two things that will surprise people: drawing is recorded rather than immediate, and a key callback's return value decides whether the byte reaches the shell.

Include the trust model in plain words — plugins are trusted, the seal catches mistakes rather than a determined bypass, and a tick that never returns will freeze the UI until M1b's budget lands.

- [ ] **Step 1: Write `docs/plugins.md`**
- [ ] **Step 2: Point the README's plugin section at it**
- [ ] **Step 3: Verify every code sample by running it as a fixture**
- [ ] **Step 4: Commit**

```bash
git add docs README.md
git commit -m "docs: the plugin ABI reference"
```

---

## Definition of done for M1a part 2

- [ ] A plugin draws text, rects, lines and markers, and they appear on screen
- [ ] `ctx:size()` and `ctx:pane_rect()` answer correctly, and a status plugin can size itself
- [ ] A tick that raises leaves the previous frame on screen, and its error is reported with the plugin's name
- [ ] A resize makes the previous frame inapplicable rather than replaying it at stale coordinates
- [ ] A stashed `ctx` cannot draw outside its tick
- [ ] A key callback returning `true` stops the byte; `false` or `nil` lets it through; the prefix reaches neither
- [ ] A second plugin binding the same key gets `nil` plus a message, both names are logged, and the first binding still works
- [ ] `pane_output` carries bytes a plugin can read
- [ ] The resize order of §4.8 holds: a `resized` subscriber and `ctx:size()` both see the new size in the same frame
- [ ] Notifications render on a host-owned surface
- [ ] The §4.11 fixture set passes as one integration test
- [ ] `idle.rs` still passes — nothing is written while nothing happens
- [ ] `docs/plugins.md` documents every call, and every sample in it runs
- [ ] `make check` and `cargo test --workspace --release` both clean

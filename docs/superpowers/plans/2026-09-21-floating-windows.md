# Floating windows — implementation plan

**Repo:** /home/kohei/ghq/github.com/Kohei-Wada/neotmux
**Spec:** `docs/superpowers/specs/2026-09-21-floating-windows-design.md` (approved)
**Date:** 2026-09-21

> **For the executor:** Required sub-skill: use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Task numbers refer to the checkboxes below. Steps use checkbox (`- [ ]`) syntax for
> tracking.

## Architecture in one paragraph

A float is **client-side only**: a grant in a small registry (`Floats`), owned by the
granting plugin. A Lua callback draws into it with `ctx:<verb>(handle, …)` in
float-local coordinates. Host-side, each draw is transformed immediately (clip to the
float's rect, offset to surface coordinates) into ordinary `DrawCmd`s and recorded in a
per-float buffer. At Tock assembly, those commands are appended after everything else,
sorted by float id — grant order, later on top. The wire, the server, and the client
render pipeline (`crates/neotmux`'s `overlay.rs`) are all left unchanged: they already
draw `pane: None` (surface) commands in list order, clipped to the surface. Keyboard
dispatch offers owners of live floats their `on_key` callbacks first, in the same
grant order, before the registration-order pass it offers today.

## File map

| File | Responsibility |
|---|---|
| `crates/neotmux-lua/src/float.rs` (new) | `FloatId`, the `Floats` registry (grant/move/resize/remove, owner, keyboard order), and the pure `place` transform (clip + offset) |
| `crates/neotmux-lua/src/api/float.rs` (new) | `neotmux.float(opts)` and the handle (`move_to`, `resize_to`, `remove`) |
| `crates/neotmux-lua/src/draw.rs` | per-float scratch/commit with the existing epoch discipline |
| `crates/neotmux-lua/src/api/ctx.rs` | target resolution in `text`/`rect`/`tint`/`line`; `ctx:float_rect` |
| `crates/neotmux-lua/src/lib.rs` | `LuaSubsystem.floats`; per-callback Ctx capture; keyboard order; `frame_output` assembly |
| `crates/neotmux-lua/src/api/mod.rs` | `Shared`/install wiring; load-failure reap |
| `crates/neotmux-lua/Cargo.toml` | `unicode-width` (text clipping) |
| `crates/neotmux-lua/tests/drawing.rs`, `tests/subsystem.rs` | the draw and keyboard contracts |
| `crates/neotmux/tests/` | detach/reattach survival |
| `docs/plugins.md`, `README.md` | the API, the rules |

`neotmux_proto` and `crates/neotmux` (the client binary) get **no new public surface**
beyond the one test fixture. `neotmux-lua` is where every production change lives.

---

## Task 1 — the registry and the transform (pure, no Lua)

**Files:**
- Create: `crates/neotmux-lua/src/float.rs`
- Modify: `crates/neotmux-lua/src/lib.rs` (one `mod float;` line)
- Modify: `crates/neotmux-lua/Cargo.toml` (add `unicode-width.workspace = true`)

- [ ] **1.1 Add the registry.** New type in `float.rs`, mirroring the shape of
  `reservations::Reservations` (RefCell inside, method surface, no public fields):

```rust
use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::source::SourceId;
use neotmux_proto::Rect;

/// A float's identity. Ids are never recycled, so the grant order is the id order.
/// The field is `pub(crate)` so the sibling modules name it by number (`__id` in Task 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FloatId(pub(crate) u32);

pub struct Floats {
    entries: RefCell<BTreeMap<FloatId, (SourceId, Rect)>>,
    next: RefCell<u32>,
}
```

Methods (all `&self`):

- `new()` — empty, first id is 1.
- `add(&self, owner: SourceId, rect: Rect) -> FloatId`
- `rect(&self, id: FloatId) -> Option<Rect>`
- `move_to(&self, id: FloatId, x: u16, y: u16)` — missing id: no-op.
- `resize(&self, id: FloatId, w: u16, h: u16)` — keeps x/y.
- `remove(&self, id: FloatId)` — idempotent.
- `by_owner(&self, owner: SourceId) -> BTreeMap<FloatId, Rect>`
- `owners_topmost_first(&self) -> Vec<SourceId>` — every owner holding a live float,
  sorted by the **largest** FloatId they hold, descending. That is the keyboard order
  (spec §5).
- `remove_all_of(&self, owner: SourceId)` — the load-failure and unload reap
  (reserve's convention).
- `count(&self) -> usize` (assertion helper).

- [ ] **1.2 Add the transform — `place`.** One pure function maps a float-local command
  to the surface command(s) it renders as. This is the whole of the clipping contract
  (spec §4: "clipped to the float's rect and to the surface"). Surface clipping is free
  — the renderer already clips `pane: None` commands to the surface — so only the
  float-local clip happens here, and every emitted command is `pane: None` at surface
  coordinates.

```rust
use unicode_width::UnicodeWidthChar;
use neotmux_proto::draw::DrawCmd;
use neotmux_proto::PColor;

/// What one float-local draw becomes at surface coordinates: clipped to the float's
/// rect, offset by its origin. A dropped draw returns an empty vec. A partially
/// visible line comes back as the runs of its steps that stayed inside.
pub fn place(cmd: &DrawCmd, float: &Rect) -> Vec<DrawCmd> {
    match cmd {
        DrawCmd::Text { x, y, text, fg, bg, .. } => clip_text(*x, *y, text, fg, bg, float),
        DrawCmd::Rect { rect, fg, bg, .. } => clip_rect(rect, *fg, *bg, float, false),
        DrawCmd::Tint { rect, fg, bg, .. } => clip_rect(rect, *fg, *bg, float, true),
        DrawCmd::Line { from, to, glyph, fg, .. } => clip_line(from, to, *glyph, *fg, float),
        // Non-exhaustive: an unknown future variant has no local meaning inside a float.
        _ => Vec::new(),
    }
}
```

`clip_text` — identical truncation rule to the renderer's `overlay.rs::draw_text`
(skip zero-width chars; a wide char that would straddle the border drops as a whole):

```rust
fn clip_text(x: u16, y: u16, text: &str, fg: &PColor, bg: &PColor, float: &Rect) -> Vec<DrawCmd> {
    if x >= float.w || y >= float.h { return Vec::new(); }
    let cap = float.w - x;
    let mut acc: u16 = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let wdt = UnicodeWidthChar::width(ch).unwrap_or(0);
        if wdt == 0 { out.push(ch); continue; }         // combining mark: rides the previous cell
        if acc.saturating_add(wdt as u16) > cap { break; }
        out.push(ch);
        acc += wdt as u16;
    }
    if out.is_empty() { return Vec::new(); }
    vec![DrawCmd::Text { pane: None, x: float.x + x, y: float.y + y, text: out, fg: fg.clone(), bg: bg.clone() }]
}
```

`clip_rect` — intersection with the float's rect, then offset:

```rust
fn clip_rect(rect: &Rect, fg, bg, float: &Rect, tint: bool) -> Vec<DrawCmd> {
    let x0 = rect.x.min(float.w);
    let y0 = rect.y.min(float.h);
    if rect.x >= float.w || rect.y >= float.h { return Vec::new(); }
    let (x1, y1) = (rect.x.saturating_add(rect.w).min(float.w), rect.y.saturating_add(rect.h).min(float.h));
    let r = neotmux_proto::Rect::new(float.x + rect.x, float.y + rect.y, x1 - rect.x, y1 - rect.y);
    let cmd = if tint { DrawCmd::Tint { pane: None, rect: r, fg, bg } } else { DrawCmd::Rect { pane: None, rect: r, fg, bg } };
    vec![cmd]
}
```

(adapt `fg`/`bg` to the owned/copy types `DrawCmd` actually carries, and the `Rect::new`
argument order, in each helper — check the struct in `neotmux_proto/src/draw.rs` and the
renderer's match arms for the exact field types.)

`clip_line` — a line is convex, so **both endpoints inside** means the whole line is
inside: offset and keep it as one `Line`. Otherwise walk the **same** Bresenham the
renderer walks (`overlay.rs::draw_line`: identical `dx`/`dy`/`sx`/`sy`/`err` stepping),
and emit one single-glyph `Text` per step that lands inside the rect — that paints the
exact same cell the renderer's `draw_char` would (symbol + fg, bg `Default`):

```rust
fn clip_line(from: &(u16, u16), to: &(u16, u16), glyph: char, fg: &PColor, float: &Rect) -> Vec<DrawCmd> {
    let inside = |p: (u16, u16)| p.0 < float.w && p.1 < float.h;
    if inside(*from) && inside(*to) {
        return vec![DrawCmd::Line { pane: None, from: (float.x + from.0, float.y + from.1), to: (float.x + to.0, float.y + to.1), glyph, fg: fg.clone() }];
    }
    let (x0, y0) = (i32::from(from.0), i32::from(from.1));
    let (x1, y1) = (i32::from(to.0), i32::from(to.1));
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    let mut out = Vec::new();
    loop {
        if x >= 0 && y >= 0
            && (x as u16) < float.w && (y as u16) < float.h {
            out.push(DrawCmd::Text {
                pane: None,
                x: float.x + x as u16,
                y: float.y + y as u16,
                text: glyph.to_string(),
                fg: fg.clone(),
                bg: PColor::Default,
            });
        }
        if x == x1 && y == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x += sx; }
        if e2 <= dx { err += dx; y += sy; }
    }
    out
}
```

- [ ] **1.3 Declare the module and the dependency.** Add `mod float;` next to the other
  private `use crate::…` / `mod …` block in `crates/neotmux-lua/src/lib.rs` (see the
  `mod` lines at the top of that file). Add `unicode-width.workspace = true` to
  `crates/neotmux-lua/Cargo.toml` (the workspace already pins `unicode-width = "0.2"`;
  the sibling crate `crates/neotmux` uses the same line).

- [ ] **1.4 Test the registry.** In `float.rs`, `#[cfg(test)] mod tests`, idioms from
  `reserve.rs`'s tests (pure, no Lua). Cases: grant order is id order; `remove` is
  idempotent and never recycles ids; `move_to`/`resize` are idempotent no-ops for a
  missing id; `by_owner` lists only that owner's floats; `remove_all_of` empties the
  owner's floats leaving other owners' untouched; `owners_topmost_first` orders by the
  **largest** id held, with an owner whose latest float is newer than another owner's
  whole history sorted first.

- [ ] **1.5 Test `place`.** Same module. Cases, one per rule:
  - text fully inside → one Text, offset x/y, same text;
  - text straddling the right edge → truncated at the width cap; a two-column char that
    would straddle drops whole (not half-drawn);
  - text starting at `x == w` or `y == h` → empty;
  - rect partially outside → clipped intersection, offset; fully outside → empty;
  - tint like rect;
  - line with both endpoints inside → one offset Line, unchanged glyph;
  - line partially outside → per-step Texts, exactly the steps inside, at offset
    coordinates, `bg: Default`.
  Use a small `Rect` (say 5x3) and a float at a non-zero origin (3, 2) so an offset
  bug is visible.

- [ ] **1.6 Run:** `cargo test -p neotmux-lua` — the new tests and every existing test
  must pass (this task changed nothing existing).

- [ ] **1.7 Commit:** `git add -A && git commit -m "the registry and placement of floating windows"`

---

## Task 2 — per-float draw buffers

**Files:** Modify: `crates/neotmux-lua/src/draw.rs`

The existing `Buffers` holds `scratch` and `committed` per `SourceId`, commits at the
frame boundary, replays committed entries only while the epoch matches, and forgets a
source wholesale. Float drawings follow the exact same epoch discipline, kept separate
because their assembly order is by float id across all owners, not by source.

- [ ] **2.1 Add the float sides.** In `Inner`, next to `scratch`/`committed`:

```rust
scratch_f: BTreeMap<SourceId, Vec<(FloatId, Vec<DrawCmd>)>>,
committed_f: BTreeMap<SourceId, (Epoch, Vec<(FloatId, Vec<DrawCmd>)>)>,
```

- [ ] **2.2 Add the methods** (mirror the existing ones line for line):
  - `record_float(&self, owner: &SourceId, id: FloatId, cmds: Vec<DrawCmd>)` — appends
    `(id, cmds)` to `scratch_f[owner]` (same "record" semantics: only a live scratch is
    visible to its owner for the rest of the tick).
  - extend `commit` (and `discard`, whatever the existing names are — read
    `draw.rs:135`'s `record` doc and its siblings): when a source's frame commits, its
    float scratch moves to `committed_f[owner]` stamped with the same epoch; when it
    errors or overruns, its float scratch is dropped, so a partial frame leaves neither
    its surface output nor its float output.
  - `forget` clears both sides for the owner (that is the whole of the unload/unsubscribe
    death for drawings).
  - `applicable_floats(&self) -> BTreeMap<FloatId, Vec<DrawCmd>>` — the union, over all
    sources, of committed float entries whose epoch matches the current one (exactly the
    way `applicable` does it for the surface side), regrouped and returned **sorted by
    FloatId** so the caller extends in grant order.

- [ ] **2.3 Test** in `draw.rs mod tests` (idioms of the existing 11 tests there; they
  build `Buffers` directly): a float recorded and committed appears in
  `applicable_floats`; a float recorded on a discarded frame (source errors) does not;
  committed float output replays for as long the epoch matches and stops when it
  changes; `forget` removes both sides; two owners with floats id 1 (owner A) and 2
  (owner B) yield `[1, 2]` ordering even if B recorded first.

- [ ] **2.4 Run:** `cargo test -p neotmux-lua`.

- [ ] **2.5 Commit:** `git commit -m "float drawings take the source's commit, per float"`

---

## Task 3 — `neotmux.float` and the handle

**Files:**
- Create: `crates/neotmux-lua/src/api/float.rs`
- Modify: `crates/neotmux-lua/src/api/mod.rs` (`pub mod float;`; a field on `Shared`;
  the install call; the load-failure reap)
- Modify: `crates/neotmux-lua/src/lib.rs` (the `LuaSubsystem` struct; its
  `build`/constructor; the `Shared { … }` passed to `api::install` at lib.rs:97)

- [ ] **3.1 The `LuaSubsystem` field.** In `lib.rs` the struct at line 55 gains
  `floats: Rc<Floats>,` (import `crate::float::Floats`); wherever `LuaSubsystem` is
  constructed (the `build` path used by `LuaSubsystem::build` in the tests) initialise
  it with `Rc::new(Floats::new())`. The api module receives the `Rc` through `Shared`
  (3.3); no extra accessor is needed.

- [ ] **3.2 The api.** `api/float.rs`, following `api/reserve.rs`'s shape
  (`install(lua, sources, registry-or-whatnot) -> mlua::Result<Function>`):

```rust
pub fn install(lua: &Lua, sources: Rc<Sources>, floats: Rc<Floats>) -> mlua::Result<Function> {
    let f = lua.create_function(move |lua, opts: Table| -> LuaResult<(Option<Table>, Option<String>)> {
        let (x, y) = corner(opts)?;                 // integer >= 0, else (nil, message)
        let (w, h) = size(opts)?;                   // integer >= 1, else (nil, message)
        let id = floats.add(source::caller(lua, &sources), Rect::new(x, y, w, h));
        let handle = lua.create_table()?;
        handle.set("__id", lua.create_function(move |_, _| Ok(id.0))?)?;
        handle.set("move_to", /* (x, y) -> (true, nil) | (nil, message): ints >= 0, floats.move_to(id, x, y) */)?;
        handle.set("resize_to", /* (w, h) -> (true, nil) | (nil, message): ints >= 1, floats.resize(id, w, h) */)?;
        handle.set("remove", lua.create_function(move |_, _| { floats.remove(id); Ok(()) })?)?;
        Ok((Some(handle), None))
    })?;
    Ok(f)
}
```

Contract (spec §2, §7): the grant is **two-value** — `local win, message =
neotmux.float{ x = 0, y = 0, w = 20, h = 5 }`; a bad grant raises nothing, records
nothing, and returns `nil` plus a human-readable message. A missing key, a non-integer,
a negative `x`/`y`, or a zero dimension is one of those. The handle methods follow the
same two-value answer (`true` + nothing, or `nil` + message) so `local ok, err =
win:move_to(3, 2)` works; `remove` is idempotent and takes no answer. `__id` is a
private field the ctx layer (Task 4) reads to resolve a handle; document it as private
in the module doc comment, the way the reserve handle's internals are.

- [ ] **3.3 Wire into the api module.** In `crates/neotmux-lua/src/api/mod.rs`:
  - add `pub mod float;` to the `mod` list (line 4-11 block);
  - add `pub(crate) floats: Rc<Floats>,` to the `Shared` struct (line 29-45);
  - in `install(lua, shared)` (line 171), next to `neotmux.set("reserve", …)` (line
    375): `neotmux.set("float", float::install(lua, shared.sources.clone(), shared.floats.clone())?)?;`
  - in `install_require_wrapper` (line 120), the load-failure branch at line 150-153
    gains, right after `reservations.remove_all_of(id);`:
    `floats.remove_all_of(id);` — and take `floats: Rc<Floats>` as its next parameter
    (update the single call site in `api::install`);
  - pass the new `Shared` field from the `api::install(…)` call at `lib.rs:97` onward
    (the `Shared { … }` construction — add `floats: self.floats.clone(),` or the
    equivalent local).

- [ ] **3.4 Test** in `crates/neotmux-lua/tests/drawing.rs` using the `built(&[…])`
  harness already there (init + plugin layer, `frame()`). A plugin's observable outputs
  are `sub.frame_output()`, the drained ops (`sub.drain_ops()`; `Op::Log` lines carry
  `neotmux.log`'s text), and Lua's return values: a plugin that wants an assertion to
  see a function's answer logs it, the way the existing tests observe plugin state.
  - a grant from an `on_tick`: `neotmux.float{ x = 2, y = 3, w = 8, h = 4 }` returns a
    table; with the rect echoed by the plugin (`neotmux.log.info` and the harness's
    log capture, or assertable return values logged the way the existing tests do).
  - a bad grant: each of a missing key, a zero `h`, a negative `x`, a non-integer `w`
    returns `nil` + a non-nil second value, and a following well-formed grant still
    works.
  - `move_to`/`resize_to` answer `true` on success and `(nil, message)` for `0` sizes /
    negative corners; `remove` twice does not raise.
  - a grant from a plugin that fails to load (the require wrapper path): no float leaks
    into a surviving plugin's draws. (Model the failing plugin on the existing
    `a_failed_loads_...` test in `tests/subsystem.rs` if one exists there.)

- [ ] **3.5 Run:** `cargo test -p neotmux-lua`.

- [ ] **3.6 Commit:** `git commit -m "neotmux.float and its handle"`

---

## Task 4 — drawing into a float, and its order

**Files:**
- Modify: `crates/neotmux-lua/src/api/ctx.rs`
- Modify: `crates/neotmux-lua/src/lib.rs` (the `Ctx::new(…)` call in `dispatch_tick`,
  lib.rs:271-279; `frame_output` at lib.rs:486)

- [ ] **4.1 `Ctx` learns its owner's floats.** The `Ctx` struct (api/ctx.rs:12)
  gains a field, and its constructor one parameter:

```rust
pub struct Ctx {
    owner: SourceId,
    buffers: Rc<Buffers>,
    size: Size,
    panes: PaneRead,
    band: Option<Rect>,
    floats: BTreeMap<FloatId, Rect>,   // this source's live floats, as of this tick
}
```

At the `Ctx::new(…)` call in `dispatch_tick` (lib.rs:269) pass
`self.floats.by_owner(owner)` (the field is right on `self`). Capturing per callback (not per frame) is deliberate
and matches `band`: a `move_to`/`resize_to` issued inside a tick takes effect on the
next frame, so the draws this tick saw clip against the rect they started with.

- [ ] **4.2 Target resolution.** The four drawing methods today take
  `Option<u32>` pane. Widen each to a `Value` first argument and resolve it:

```rust
enum Target { Surface, Pane(u32), Float(FloatId) }

fn resolve(t: &Value) -> LuaResult<Target> {
    match t {
        Value::Nil => Ok(Target::Surface),
        Value::Integer(i) => Ok(Target::Pane(*i as u32)),
        Value::Table(tbl) => {
            let id_fn: Function = tbl.get("__id")?;
            let id: u32 = id_fn.call(())?;
            Ok(Target::Float(FloatId(id)))
        }
        other => Err(luatype...::Error::new(TypeError, format!("a pane, a float, or nil is expected, not {other:?}"))),
    }
}
```

(match the file's existing error idiom for a wrong-typed argument — read the current
signatures' behaviour first)

In each of `text`, `rect`, `tint`, `line`, keep the Surface and Pane arms byte-for-byte
what they are today (record the same `DrawCmd` as now), and add the Float arm:

```rust
Target::Float(fid) => {
    let cmd = /* the same DrawCmd that would have been recorded, pane-local, as if pane: None at (x, y) … */;
    if let Some(r) = self.floats.get(&fid) {
        self.buffers.record_float(&self.owner, fid, crate::float::place(&cmd, r));
    }
    // else: this float closed in the same tick — the closed-pane convention, drop the draw
}
```

(`cmd` is built exactly as the Surface arm builds it, but always with `pane: None` and
local coordinates; `place` then clips and offsets. For `text` that is `DrawCmd::Text {
pane: None, x, y, text, fg, bg }` — `place`'s clip handles `x >= w` drops, so no
special case here.)

- [ ] **4.3 `ctx:float_rect(win)`**. A new `Ctx` method: resolve a float target (a
  plain number or nil here is a type error — it is for a float, not a pane), look it up
  in the captured `self.floats`, and return the surface rect as a Lua table
  `{ x, y, w, h }` (the same table convention `ctx:size` uses for the surface), or
  `nil` when the source no longer owns the float.

- [ ] **4.4 The assembly.** `frame_output` (lib.rs:486-489) appends the float output
  after everything else, in float-id order:

```rust
pub fn frame_output(&self) -> Vec<DrawCmd> {
    let mut out = self.buffers.applicable();
    for cmds in self.buffers.applicable_floats().values() {
        out.extend(cmds);
    }
    out
}
```

This is the whole of the layering contract (spec §4 drawing order): panes and bands
render before any overlay command, and overlays render in this list order, so every
float sits above every non-float overlay command and the later grant sits above the
earlier one.

- [ ] **4.5 Test** in `crates/neotmux-lua/tests/drawing.rs` (same harness; plugins log
  nothing — assert `sub.frame_output()` directly, `DrawCmd` is `pub`):
  - **offset and clip, per verb.** A plugin grants a float at (3, 2) sized 8x4, then
    draws `text`, `rect`, `tint`, and `line` into it at local (1, 1):
    `frame_output()` contains each transformed command at the surface coordinates (4, 3
    etc.), and a text that runs past the float's right edge arrives truncated to
    exactly the float-local width cap, not full-length.
  - **surface clip is the renderer's job.** A float at (98, 20) on the 80x24 surface in
    the existing harness: the commands still arrive whole (offset, float-clipped) — the
    harness does not render, which is correct: only the float clip is host-side.
  - **grant order, not draw order, is z.** Two plugins: A loads first and draws first;
    B grants its float *at load time* (before A grants, A grants inside its tick): B's
    float is id 1, A's id 2; `frame_output()` lists A's float commands **after** B's,
    even though B's plugin drew first this tick.
  - **a draw into a float the source no longer owns is dropped.** Grant inside tick 1,
    `remove()` inside the same tick *after* the own draw, then a second `on_tick`
    callback of the same source drawing into the same handle: the first draw arrives,
    the second does not (per-callback capture, per 4.1).
  - **`ctx:float_rect`**: returns `{x=3, y=2, w=8, h=4}` for the live float (assert by
    having the plugin `neotmux.log.info` the four fields and reading the harness log,
    the way the `clock.lua`/`splitter.lua` style tests are asserted); `nil` after
    `remove`.
  - **the canary.** A session whose plugins hold no floats (the existing
    `a_tick_records_what_it_draws` test) still produces exactly its one command, at the
    same coordinates, as before this step — re-run the whole file.

- [ ] **4.6 Run:** `cargo test -p neotmux-lua`.

- [ ] **4.7 Commit:** `git commit -m "ctx draws into a float, above everything else, in grant order"`

---

## Task 5 — the keyboard: float owners first

**Files:** Modify: `crates/neotmux-lua/src/lib.rs`

- [ ] **5.1 The new order.** `dispatch_key` (lib.rs:362) currently runs
  `offer_key` (all `on_key` callbacks in registration order, lib.rs:374) then
  `dispatch_binding` (lib.rs:397).
  Refactor `offer_key`'s body into a single-callback offer — e.g.

```rust
/// One callback, its consume answer, its Lua errors.
fn offer_one(&self, id: &RegistrationId, normalised: &str) -> bool { … }
```

and rewrite `dispatch_key` to the spec §5 order:

```rust
pub fn dispatch_key(&self, normalised: &str) -> KeyOutcome {
    // First, every owner holding a live float, topmost grant first: their on_key
    // callbacks, registration order inside the owner.
    let floats_first: BTreeSet<SourceId> = self.floats.owners_topmost_first().into_iter().collect();
    for owner in self.floats.owners_topmost_first() {
        for id in self.registry.snapshot(KindTag::Key) {
            if self.registry.owner(&id) != Some(owner) { continue; }
            if self.offer_one(&id, normalised) { return KeyOutcome::Consumed; }
        }
    }
    // Then the rest, in the registration order the canary knows.
    for id in self.registry.snapshot(KindTag::Key) {
        if let Some(owner) = self.registry.owner(&id) {
            if floats_first.contains(&owner) { continue; }   // already offered above
        }
        if self.offer_one(&id, normalised) { return KeyOutcome::Consumed; }
    }
    // Then the bindings, then the socket (the existing tail, unchanged).
    self.dispatch_binding(normalised)
}
```

(adjust `owner(&id)` / `snapshot` / `RegistrationId` to the actual accessors — read
`offer_key` first; `KeyOutcome` is `crate::KeyOutcome`.) The invariant a reviewer
checks: with `owners_topmost_first()` empty the executed callback order is exactly
today's (the canary).

- [ ] **5.2 The `fallthrough` is untouched.** `KeyOutcome::Fallthrough` still reaches
  `dispatch_binding` and then the existing Tail → `ToServer::Key` → focused pane. No
  change to `key_spec` normalisation (arrow keys stay CSI — issue #5, out of scope).

- [ ] **5.3 Test** in `crates/neotmux-lua/tests/subsystem.rs` (harness idioms of the
  existing tests there — two plugins, `neotmux.on_key(function(key) … return true end)`,
  `sub.dispatch_key(…)` — read one existing test for the frame/key plumbing):
  - **float beats registration order.** A registers on_key first (consumes `x`), B
    second (consumes `x`); A grants no float, B grants one (at load). `x` → B consumes.
    With no floats at all (control subtest), `x` → A consumes. That pair is the
    canary.
  - **removal restores the canary.** B's `x` callback, on its turn, `remove()`s B's
    float; the next `x` press → A consumes again.
  - **topmost float among float-holders wins.** B and C both hold floats (`x` key); C's
    is newer → C consumes.
  - **a float without an on_key steals nothing.** F holds a float, no key callbacks; K
    consumes `x`; with `x`, K wins whether or not F is loaded.
  - **a float-owner's binding still waits behind every on_key.** F holds a float and a
    binding for `x`; K (no float) has an on_key for `x`. `x` → K's callback, not F's
    binding.

- [ ] **5.4 Run:** `cargo test -p neotmux-lua`.

- [ ] **5.5 Commit:** `git commit -m "the float owners, topmost first, hold the keys before the rest"`

---

## Task 6 — the integration seam: detach, reattach, and the canary

**Files:**
- Create: `crates/neotmux/tests/fixtures/lua/plugin/float_probe.lua`
- Modify: `crates/neotmux/tests/socket.rs` (one new test, next to
  `a_second_client_is_shown_the_session_the_first_one_left`)
- (The `Neotmux` harness in `crates/neotmux/tests/fixtures.rs` and `socket.rs`'s
  `layer_with`, `wait_for_log` helpers are reused as-is.)

- [ ] **6.1 The probe plugin.** Loads a float the way a real one does and announces,
  every tick while it is alive and each time it moves, what the client claims to own —
  enough to see through the socket boundary:

```lua
-- The client's side of the float, announced each tick so a test can watch a reattach.
local win = nil
neotmux.on_tick(function(ctx)
  if not win then win = neotmux.float{ x = 4, y = 3, w = 12, h = 4 } end
  if win then
    ctx:text(win, 0, 0, "float window")
    local r = ctx:float_rect(win)
    if r then
      neotmux.log.info("float " .. r.x .. "," .. r.y .. " " .. r.w .. "x" .. r.h)
    end
  end
end)
return {}
```

(the log line only exists while a client-side float is alive — a reattach must start
it again from scratch, and that is exactly what the test is watching).

- [ ] **6.2 The test.** In `socket.rs`, beside the existing detach/reattach tests:

```rust
#[test]
fn a_reattached_client_builds_the_same_float_again() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[("lua/plugin/float_probe.lua", "<the probe, 6.1>")]);
    // client 1, with the probe: its float is announced every frame.
    let mut first = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", &layer);
    first.wait_for_output("float 4,3 12x4");
    // the seam: the server outlives the client, and client 1's text stays on the shared pane.
    first.send("still-under-the-float\r");
    first.wait_for_output("still-under-the-float");
    first.kill();
    drop(first);
    // client 2, same probe: the plugin layer re-runs from scratch and the float is back
    // over the same session — a float is client state, and this test is its proof.
    let second = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", &layer);
    second.wait_for_output("float 4,3 12x4");
    second.wait_for_output("still-under-the-float");
}
```

(adjust the local helpers to this file's actual names — `TestRuntime`, `Client::start`,
`layer_with`, socket.rs:30-240; the shape is `a_second_client_is_shown_the_session_the_first_one_left`,
socket.rs:322-336, with a layer on both clients. The assertion set is the point: the
probe's `float 4,3 12x4` line comes back, and client 1's text is still under it.)

- [ ] **6.3 The no-float canary through the real socket.** Run
  `cargo test -p neotmux socket` — every pre-existing test (especially the detach ones)
  is untouched and green; they are the proof that a session with no floats is
  byte-for-byte the old behaviour through the real protocol.

- [ ] **6.4 Run:** `cargo test -p neotmux` (the whole integration suite).

- [ ] **6.5 Commit:** `git commit -m "a reattached client rebuilds its floats over the surviving session"`

---

## Task 7 — the docs and the full pass

**Files:** Modify: `docs/plugins.md`, `README.md`; verify: spec §9's list line for line.

- [ ] **7.1 `docs/plugins.md`.** Four touch points, in the file's own voice (every
  paragraph lands in the register of its section — read the section first):
  - the registration API table in `### Registering` (lines 59-93, the row beside
    `neotmux.on_key`): one row for `neotmux.float{ x = , y = , w = , h = }` and its
    two-value answer;
  - `### Drawing` (line 94): the float as a third drawing target next to `nil` and a pane
    id — in the opening example and in the "Coordinates are the surface's." paragraph:
    float-local coordinates, clipped to the float, then to the surface; and
    `ctx:float_rect(win)` documented beside `ctx:reserved()`;
  - a new `###` section right after `### Reserving a band` (lines 158-219 — model its
    shape clause for clause): the grant, `move_to`, `resize_to`, `remove`; `__id` as
    private; the lifetime paragraph (dies on the owner's unload or unsubscribe, not on
    the frame; a reattach rebuilds it) with the one keyboard sentence (**the owners of
    live floats are offered every key before the rest of the plugin layer, topmost float
    first**; spec §5 — one sentence, not a rules paragraph); "a float takes no cells" and
    "the server never knows it exists" each in one sentence, as the band section does;
  - `## When things break` (line 671): the two-value error rows for the float grant and
    the handle methods, beside the band's rows.

  No other section changes: `### Windows` (line 508) and the prefix/mode sections stay
  untouched — say so in the commit body if the diff is tempted otherwise.

- [ ] **7.2 `README.md`.** Line 26, where the plugin layer is introduced ("lets it
  register tick, event and keymap callbacks, reserve a band"), gains "and float a window
  over the panes" in the same sentence; and the v0.1 surface list (lines 35-36) names it
  beside the band, with one pointer to the plugins.md section.

- [ ] **7.3 Spec conformance.** Read `docs/superpowers/specs/2026-09-21-floating-windows-design.md`
  top to bottom against the code: every §9 test exists and passes; every §7 error row
  has a test; §5's order sentence matches `dispatch_key`; §8's "no protocol change" is
  true by construction (say it in the commit message if Task 6 added no wire fields).

- [ ] **7.4 The full gate.** `cargo test --workspace` (all green, including the
  pre-existing 700+), `cargo clippy --workspace --all-targets` (clean),
  `cargo fmt --all -- --check` (the repo's formatter pass — verify with
  `git config core.hooksPath`-independent `cargo fmt --check` if that differs).

- [ ] **7.5 Commit:** `git commit -m "the documentation and the pass of the conformance"`

---

## Verification (end state)

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/neotmux
cargo test --workspace           # all green
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
git log --oneline a316013..HEAD    # the six feature commits + docs, on top of the spec commit
```

Outcomes an executor reports back: the six commit SHAs; the count of new tests per
crate; the clippy/fmt status; any place a spec sentence and the code diverged (with
which sentence won).

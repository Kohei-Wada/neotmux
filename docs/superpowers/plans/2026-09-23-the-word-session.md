# The Word "Session" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The server and the Lua layer learn which session a piece of work is for, while there
is still exactly one of them.

**Architecture:** A `SessionId` joins `PaneId` and `WindowId` in `neotmux-proto`. The server
holds `BTreeMap<SessionId, Session>` with one entry and routes every frame, key and resize
through it. The Lua layer gains an ambient current session: the two facts that are genuinely
per-session — the geometry and the pane info — move into a store, and binding a session swaps
that session's into the live cells the API's closures already hold, so not one closure changes.
Ops are tagged with the session they were raised for.

**Tech Stack:** Rust 2024, one workspace of five crates, `mlua` (Lua 5.4).

**Spec:** `docs/superpowers/specs/2026-09-23-sessions-in-the-server-design.md`

## Why this is not the spec's stage 1

The spec's §11 opens with the socket layer: one server, one socket, a `Sessions` map with one
entry. That order has a hole in it. `crates/neotmux/tests/sessions.rs:158-200` starts `work`
and `default` **at the same time** and pins that `ls` lists both and that `kill work` leaves
`default` running. Consolidating the socket while the map can hold one entry breaks that, and
it stays broken until the whole of the spec's stage 2 has landed.

So the stages are reordered, and the spec is amended to say so (Task 6):

1. **This plan.** The internals learn the word, with one session. Nothing observable changes;
   no test changes.
2. The socket, the CLI and N sessions, together, so the two-name behaviour moves from "two
   servers" to "two sessions in one server" in one step rather than being absent in between.
3. `neotmux.session.*`, including `with`.

## What the spec over-estimated

§6 lists six pieces of `LuaSubsystem` state as needing to become per-session. Reading them
says two do:

| state | per session? | why |
| --- | --- | --- |
| `geometry` | **yes** | what the panes and windows of *that* session are |
| `pane_info` | **yes** | what those panes are running |
| `reservations` | no | already split into a declaration (`add`/`totals`) and a per-frame placement (`lay_out(surface, applied)`, `band(owner)`). §7's rule is the shape it already has |
| `floats` | no | `entries: BTreeMap<FloatId, (SourceId, Rect)>` is the declaration; where it lands is computed against a surface at draw time |
| `buffers` | no | scratch for the dispatch in hand: `begin`, `record`, `commit`, `applicable` all happen inside one session's tick |
| `window_ids` | no | a counter. Server-wide makes ids unique across sessions too, which is free and is what a later `$1:@3` display would want |

This is worth stating rather than quietly doing less work than the spec asked for: the reason
four of the six need nothing is that the reservation and float APIs were already built as
declarations, which is the same insight §7 arrived at from the other end.

## Global Constraints

- `neotmux-lua` keeps `neotmux-proto` as its only workspace dependency, beside `anyhow` and
  `mlua`.
- **No observable behaviour changes in this plan.** Not one test file is edited. If a test
  needs changing, the change is wrong — stop and say so.
- Exactly one session exists at runtime throughout. Nothing here creates a second one; the
  map, the ids and the ambient binding are the shape that will hold several.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` green at every commit, read rather than assumed.
- English for code, comments and commits. Comments only where the WHY is non-obvious.
- Commit messages in this repo's voice, ending with the two attribution lines recent commits
  carry.

---

## File Structure

| File | Responsibility afterwards |
| --- | --- |
| `crates/neotmux-proto/src/ids.rs` | `SessionId`, beside `PaneId` and `WindowId` |
| `crates/neotmux-lua/src/ambient.rs` | **New.** Every session's geometry and pane info, and which one is bound |
| `crates/neotmux-lua/src/lib.rs` | Dispatch entry points name a session and bind it |
| `crates/neotmux-lua/src/ops.rs` | An op remembers the session it was raised for |
| `crates/neotmux/src/server.rs` | `BTreeMap<SessionId, Session>`, one entry; a client names one |
| `crates/neotmux/src/plugins.rs` | Its three entry points take the session |

---

### Task 1: `SessionId`

**Files:**
- Modify: `crates/neotmux-proto/src/ids.rs`
- Modify: `crates/neotmux-proto/src/lib.rs` (the re-export beside `PaneId`, `WindowId`)

**Interfaces:**
- Produces: `neotmux_proto::SessionId`, with `From<u32>`, `get()`, `Display` as `$n`, and the
  same derives the other two ids carry.

- [ ] **Step 1: Write the failing test**

Into `ids.rs`'s `mod tests`:

```rust
    #[test]
    fn a_session_id_is_written_the_way_tmux_writes_one() {
        let id: SessionId = 1u32.into();
        assert_eq!(id.get(), 1);
        assert_eq!(id.to_string(), "$1");
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux-proto ids`
Expected: FAIL, `cannot find type 'SessionId' in this scope`.

- [ ] **Step 3: Implement**

Beside `WindowId`, following it exactly:

```rust
/// One session: a set of windows, the thing a client attaches to.
///
/// `WindowId`'s note above called this addition purely additive and it is: a window keeps
/// meaning what it meant, and this sits above it. The sigil is tmux's, which spells a pane
/// `%1`, a window `@1` and a session `$1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u32);

impl SessionId {
    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SessionId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.0)
    }
}
```

Export it from `lib.rs` wherever `WindowId` is exported — read that line and add to it rather
than writing a new one.

- [ ] **Step 4: Run it**

Run: `cargo test -p neotmux-proto ids`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-proto/src/ids.rs crates/neotmux-proto/src/lib.rs
git commit -m "a session is a thing with an id"
```

---

### Task 2: The ambient session

The two per-session facts move into a store. The live cells stay exactly where they are and
keep their types, so `api/pane.rs`, `api/window.rs` and `api/ctx.rs` — which capture
`Rc<RefCell<Geometry>>` in a dozen closures — are not touched at all. Binding a session swaps
its stored values into the live cells and puts the previous ones back on the way out.

The swap is `std::mem::swap` on a `Geometry` and a `BTreeMap`, which moves their headers. No
clone, whatever a session's geometry holds.

**Files:**
- Create: `crates/neotmux-lua/src/ambient.rs`
- Modify: `crates/neotmux-lua/src/lib.rs` (the module list; the two fields)

**Interfaces:**
- Consumes: `SessionId` (Task 1).
- Produces: `crate::ambient::Ambient`, with
  `Ambient::new(geometry: Rc<RefCell<Geometry>>, pane_info: Rc<RefCell<PaneInfos>>) -> Ambient`,
  `Ambient::current(&self) -> Option<SessionId>`,
  `Ambient::bind(&self, session: SessionId) -> Bound<'_>`, and `Bound` restoring on drop.

- [ ] **Step 1: Write the failing tests**

Create `crates/neotmux-lua/src/ambient.rs` with the module doc and the tests only:

```rust
//! Which session the work in hand is for, and the facts that are that session's.
//!
//! There is one Lua layer for the server, so the geometry a plugin reads has to be the
//! geometry of the session whose tick is running. The API's closures were written against one
//! `Rc<RefCell<Geometry>>` and there is no reason to rewrite them: that cell is the *current*
//! session's, this holds everybody else's, and binding a session swaps the two. The swap moves
//! a `Vec`'s and a `BTreeMap`'s headers, so it costs the same whatever a session holds.

#[cfg(test)]
mod tests {
    use super::*;
    use neotmux_proto::Size;

    fn ambient() -> (Ambient, Rc<RefCell<Geometry>>) {
        let geometry = Rc::new(RefCell::new(Geometry::default()));
        let pane_info = Rc::new(RefCell::new(PaneInfos::new()));
        (Ambient::new(geometry.clone(), pane_info), geometry)
    }

    fn sized(rows: u16) -> Geometry {
        Geometry {
            size: Size { rows, cols: 80 },
            ..Geometry::default()
        }
    }

    #[test]
    fn nothing_is_bound_until_something_binds_it() {
        let (ambient, _) = ambient();
        assert_eq!(ambient.current(), None);
    }

    /// What a session was left holding is what it is given back, and the live cell is
    /// whichever session is bound right now.
    #[test]
    fn each_session_keeps_its_own_geometry_across_a_bind() {
        let (ambient, live) = ambient();
        let one = SessionId::from(1);
        let two = SessionId::from(2);

        {
            let _bound = ambient.bind(one);
            *live.borrow_mut() = sized(10);
        }
        {
            let _bound = ambient.bind(two);
            assert_eq!(
                live.borrow().size.rows,
                0,
                "session two was shown session one's geometry"
            );
            *live.borrow_mut() = sized(20);
        }
        let _bound = ambient.bind(one);
        assert_eq!(live.borrow().size.rows, 10);
    }

    #[test]
    fn a_bind_inside_a_bind_puts_the_outer_one_back() {
        let (ambient, live) = ambient();
        let outer = SessionId::from(1);
        let inner = SessionId::from(2);

        let _bound = ambient.bind(outer);
        *live.borrow_mut() = sized(10);
        {
            let _inner = ambient.bind(inner);
            assert_eq!(ambient.current(), Some(inner));
            *live.borrow_mut() = sized(20);
        }
        assert_eq!(ambient.current(), Some(outer));
        assert_eq!(live.borrow().size.rows, 10);
    }

    /// The guard is what restores, so an early return or an error on the way out of a
    /// dispatch cannot leave another session's facts in the live cell.
    #[test]
    fn dropping_the_guard_restores_even_when_the_work_did_not_finish() {
        let (ambient, live) = ambient();
        let one = SessionId::from(1);
        {
            let _bound = ambient.bind(one);
            *live.borrow_mut() = sized(10);
        }
        assert_eq!(ambient.current(), None);
        assert_eq!(
            live.borrow().size.rows,
            0,
            "the unbound cell still held a session's geometry"
        );
    }
}
```

- [ ] **Step 2: Declare the module and watch it fail**

Add `mod ambient;` to `crates/neotmux-lua/src/lib.rs` beside the other private modules.

Run: `cargo test -p neotmux-lua --lib ambient`
Expected: FAIL, `cannot find type 'Ambient' in this scope`.

- [ ] **Step 3: Implement**

Above the test module:

```rust
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use neotmux_proto::{Geometry, SessionId};

use crate::paneinfo::PaneInfos;

/// What one session is holding while some other session is the one bound.
#[derive(Default)]
struct Parked {
    geometry: Geometry,
    pane_info: PaneInfos,
}

pub struct Ambient {
    /// The cells the API's closures hold. Whatever is in them belongs to `current`.
    geometry: Rc<RefCell<Geometry>>,
    pane_info: Rc<RefCell<PaneInfos>>,
    current: Cell<Option<SessionId>>,
    parked: RefCell<BTreeMap<SessionId, Parked>>,
}

impl Ambient {
    pub fn new(geometry: Rc<RefCell<Geometry>>, pane_info: Rc<RefCell<PaneInfos>>) -> Self {
        Self {
            geometry,
            pane_info,
            current: Cell::new(None),
            parked: RefCell::new(BTreeMap::new()),
        }
    }

    /// The session the work in hand is for, or `None` outside any dispatch — during a config
    /// load, for instance, where there is no session yet.
    pub fn current(&self) -> Option<SessionId> {
        self.current.get()
    }

    /// Makes `session` the one the live cells belong to, until the guard is dropped.
    pub fn bind(&self, session: SessionId) -> Bound<'_> {
        self.swap(Some(session));
        Bound { ambient: self }
    }

    /// Puts what the live cells hold back where it came from and brings out `next`'s.
    ///
    /// `None` parks under no id, which is what an unbound layer holds: a config load draws on
    /// an empty geometry and must not be handed some session's.
    fn swap(&self, next: Option<SessionId>) {
        let was = self.current.replace(next);
        let mut parked = self.parked.borrow_mut();
        let mut out = Parked::default();
        std::mem::swap(&mut out.geometry, &mut self.geometry.borrow_mut());
        std::mem::swap(&mut out.pane_info, &mut self.pane_info.borrow_mut());
        if let Some(was) = was {
            parked.insert(was, out);
        }
        if let Some(next) = next
            && let Some(mut theirs) = parked.remove(&next)
        {
            std::mem::swap(&mut theirs.geometry, &mut self.geometry.borrow_mut());
            std::mem::swap(&mut theirs.pane_info, &mut self.pane_info.borrow_mut());
        }
    }

    /// Everything a session was holding, dropped: it is over.
    pub fn forget(&self, session: SessionId) {
        self.parked.borrow_mut().remove(&session);
    }
}

/// The binding, undone when it goes out of scope — including on the way out of a dispatch
/// that ended in an error, which is the whole reason it is a guard and not two calls.
///
/// It restores to what was bound before rather than to nothing, so a bind inside a bind — a
/// plugin reading another session from inside its own tick — leaves its caller where it was.
pub struct Bound<'a> {
    ambient: &'a Ambient,
    was: Option<SessionId>,
}

impl Drop for Bound<'_> {
    fn drop(&mut self) {
        self.ambient.swap(self.was);
    }
}
```

and `bind` reads the previous id before it swaps:

```rust
    pub fn bind(&self, session: SessionId) -> Bound<'_> {
        let was = self.current.get();
        self.swap(Some(session));
        Bound {
            ambient: self,
            was,
        }
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --lib ambient`
Expected: PASS, all four.

- [ ] **Step 5: Hold it in `LuaSubsystem`**

In `lib.rs`, build the `Ambient` from the two cells that already exist and keep both — the
subsystem holds the `Rc`s for its own `republish_geometry`/`publish_pane_info`, and the
`Ambient` for binding:

```rust
        let geometry = Rc::new(RefCell::new(Geometry::default()));
        let pane_info = Rc::new(RefCell::new(PaneInfos::new()));
        let ambient = Rc::new(Ambient::new(geometry.clone(), pane_info.clone()));
```

with `ambient: Rc<Ambient>` added to the struct and to its constructor's tail. Nothing reads it
yet; Task 3 does.

- [ ] **Step 6: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS, with no test file edited.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux-lua/src/ambient.rs crates/neotmux-lua/src/lib.rs
git commit -m "somewhere for a session that is not the one in hand"
```

---

### Task 3: Dispatch names its session

Every entry point into the Lua layer that is about a session takes one and binds it. There is
one session, so every call passes the same id and nothing changes.

**Files:**
- Modify: `crates/neotmux-lua/src/lib.rs`
- Modify: `crates/neotmux-lua/src/ops.rs`
- Modify: `crates/neotmux/src/plugins.rs`
- Modify (callers of the changed methods): `crates/neotmux-lua/tests/*.rs`, which drive the
  subsystem directly and are the only tests that name these methods. Passing a session where
  they passed none is not a behaviour change, and the global constraint about not editing
  tests is about what they assert, not about a signature they call.
- Test: `crates/neotmux-lua/tests/panes.rs`

**Interfaces:**
- Consumes: `Ambient` (Task 2).
- Produces:
  - `LuaSubsystem::begin_frame(&self, session: SessionId, epoch: Epoch, geometry: Geometry)`
  - `LuaSubsystem::republish_geometry(&self, session: SessionId, geometry: Geometry)`
  - `LuaSubsystem::publish_pane_info(&self, session: SessionId, info: BTreeMap<PaneId, PaneInfo>)`
  - `LuaSubsystem::dispatch_tick(&self, session: SessionId)`
  - `LuaSubsystem::dispatch_key(&self, session: SessionId, spec: &str) -> KeyOutcome`
  - `LuaSubsystem::dispatch_event(&self, session: SessionId, event: &Event)`
  - `LuaSubsystem::lay_out_reservations(&self, session: SessionId, surface: Size, applied: Reserved)`
  - `LuaSubsystem::drain_ops(&self, session: Option<SessionId>) -> Vec<(SourceId, Op)>` —
    `None` is the load path, which runs before any session exists
  - `LuaSubsystem::forget_session(&self, session: SessionId)`
  - `Plugins::respond(&mut self, session: SessionId, tick: Tick) -> Tock`,
    `Plugins::key(&self, session: SessionId, key: Key) -> KeyOutcome`,
    `Plugins::catch_up(&self, session: SessionId, steps: Vec<Step>)`

- [ ] **Step 1: Write the failing test**

`crates/neotmux-lua/tests/panes.rs` already has the helpers this needs: `built(files)`,
`publish(sub, panes, focus)` and `logged(sub)`. `publish` and `logged` gain a session
parameter and every existing call site in that file passes `one`, declared beside them:

```rust
/// The one session every test in this file drives. They are about panes rather than about
/// sessions, and a session is what a geometry now belongs to.
const ONE: SessionId = SessionId::from(1);
```

`SessionId::from` is not `const`, so declare it as a function instead if the compiler says so:

```rust
fn one() -> SessionId {
    SessionId::from(1)
}
```

Then the new test, which proves both halves of this task at once — the geometry a tick reads
and the session an op is raised for:

```rust
/// One Lua layer, two sessions: what a tick reads is the geometry of the session whose tick
/// it is, and what it raises belongs to that session. Nothing in the program makes a second
/// session yet — the layer is where the shape is proved first.
#[test]
fn a_tick_reads_and_raises_against_the_session_it_is_for() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function() neotmux.log.info('panes ' .. #neotmux.pane.list()) end)",
    )]);
    let two = SessionId::from(2);
    let wide = vec![
        (PaneId::from(1), Rect::new(0, 0, 40, 24)),
        (PaneId::from(2), Rect::new(40, 0, 40, 24)),
    ];
    let lone = vec![(PaneId::from(3), Rect::new(0, 0, 80, 24))];

    publish(&sub, one(), wide.clone(), PaneId::from(2));
    sub.dispatch_tick(one());
    publish(&sub, two, lone, PaneId::from(3));
    sub.dispatch_tick(two);
    publish(&sub, one(), wide, PaneId::from(2));
    sub.dispatch_tick(one());

    assert_eq!(
        logged(&sub, Some(one())),
        vec!["panes 2".to_string(), "panes 2".to_string()],
        "session one's ticks saw session two's geometry, or its ops went to the wrong session"
    );
    assert_eq!(logged(&sub, Some(two)), vec!["panes 1".to_string()]);
}
```

with the two helpers taking what they now need:

```rust
fn publish(sub: &LuaSubsystem, session: SessionId, panes: Vec<(PaneId, Rect)>, focus: PaneId) {
    let size = Size { rows: 24, cols: 80 };
    sub.begin_frame(
        session,
        Epoch::new(size, panes.iter().map(|(id, _)| *id).collect(), Some(focus)),
        Geometry {
            size,
            panes,
            focus: Some(focus),
            ..Default::default()
        },
    );
}

fn logged(sub: &LuaSubsystem, session: Option<SessionId>) -> Vec<String> {
    sub.drain_ops(session)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux-lua --test subsystem a_tick_sees_the_geometry`
Expected: FAIL to compile — the methods take no session.

- [ ] **Step 3: Thread the session through**

Each of the methods above takes `session: SessionId` as its first parameter and opens with

```rust
        let _bound = self.ambient.bind(session);
```

`republish_geometry` and `publish_pane_info` write through the live cells as they do now, and
the binding is what makes that the right session's. `begin_frame` binds once and calls
`republish_geometry`'s body rather than the method, or binds twice — nesting is supported, so
either is correct; prefer whichever reads better.

`forget_session` calls `self.ambient.forget(session)`.

- [ ] **Step 4: Tag the ops**

An op raised during session S's dispatch is applied to S. The buffer reads the ambient session
rather than every call site passing it — there are a dozen `ops.push` sites in `api/`, and none
of them knows or should know:

```rust
pub struct OpBuffer {
    ops: RefCell<Vec<(Option<SessionId>, SourceId, Op)>>,
    ambient: Rc<Ambient>,
}
```

`push` records `self.ambient.current()`. `drain` becomes:

```rust
    /// What was raised for this session, taken out and nothing else. `None` is the load path:
    /// an op raised before any session existed belongs to no session, and `drain_load_ops` is
    /// the one caller that asks for those.
    pub fn drain_for(&self, session: Option<SessionId>) -> Vec<(SourceId, Op)> {
        let mut ops = self.ops.borrow_mut();
        let mut mine = Vec::new();
        ops.retain(|(raised, owner, op)| {
            if *raised == session {
                mine.push((*owner, op.clone()));
                false
            } else {
                true
            }
        });
        mine
    }
```

`mark`/`discard_from` keep working on positions and owners as they do; read them and keep
their contract — a discarded frame takes its own ops back out, and that is not about sessions.

`OpBuffer::new()` gains the `Rc<Ambient>`, so `lib.rs` builds the ambient before the buffer.

- [ ] **Step 5: `Plugins` passes it on**

`respond`, `key` and `catch_up` take a `SessionId` and hand it to each call. `drain_ops` inside
`respond` becomes `drain_ops(session)`; `drain_load_ops` drains the unbound ones.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p neotmux-lua`
Expected: PASS, the new one included.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux-lua crates/neotmux/src/plugins.rs
git commit -m "a dispatch says which session it is for"
```

---

### Task 4: The server holds a map

**Files:**
- Modify: `crates/neotmux/src/server.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: no new public API. Internally, `Sessions` replacing `Option<Session>`, and
  `Attached.session: SessionId`.

- [ ] **Step 1: The map**

```rust
/// Every session this server holds, and the ids to come. One entry today; the map is the
/// shape the rest of them go in.
#[derive(Default)]
struct Sessions {
    live: BTreeMap<SessionId, Session>,
    next: u32,
}

impl Sessions {
    /// The id a new session takes. Never recycled, so an id names one session for the life of
    /// the server.
    fn mint(&mut self) -> SessionId {
        self.next += 1;
        SessionId::from(self.next)
    }
}
```

`frame`'s `let Some(live_session) = session.as_mut() else { ... }` becomes a loop over
`sessions.live` — with one entry, one pass. Each pass binds that session's id through
`plugins.respond(id, tick)` and `plugins.catch_up(id, steps)`.

`Attached` gains `session: SessionId`, set in `admit`. A client's picture is composed from the
session its `session` names, which today is the only one.

`deliver`'s `Key` arm passes `clients[i].session` to `plugins.key(...)` and applies the
fallthrough to that session; `Resize` resizes that session.

- [ ] **Step 2: A session that ends is forgotten**

Where a session ends — `End::Eof`, or a `Quit` — call `plugins.lua().forget_session(id)` and
remove it from the map, so a long-lived server does not keep a dead session's geometry parked
for ever. With one session the server exits anyway; the call is what makes that true later too,
and it is one line in the right place rather than a thing to remember.

- [ ] **Step 3: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS. **No test file is edited.** `socket.rs`, `sessions.rs`, `fixtures.rs` and the
rest all pass as they stand, which is the claim this whole plan makes.

- [ ] **Step 4: Commit**

```bash
git add crates/neotmux/src/server.rs
git commit -m "the server holds sessions, and today it holds one"
```

---

### Task 5: The load path has no session

A config load runs before any session exists, and `init.lua` legitimately calls
`neotmux.reserve` and `neotmux.float` there. Nothing binds a session around `run_init`, so
`Ambient::current()` is `None` throughout — which is what Task 3's `drain_for(None)` relies on.

This task is one test, because the property is easy to break later and impossible to see.

**Files:**
- Test: `crates/neotmux-lua/tests/subsystem.rs`

- [ ] **Step 1: Write the test**

In `crates/neotmux-lua/tests/panes.rs`, beside the test from Task 3 and using the same
`built` and `logged`:

```rust
/// A load happens before any session does. What it reads is an empty geometry rather than
/// some session's — there is not one yet — and what it raises belongs to no session, which is
/// the case `drain_ops(None)` exists for and the one `Plugins::drain_load_ops` asks.
#[test]
fn a_config_load_runs_with_no_session_bound() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.log.info('panes at load ' .. #neotmux.pane.list())",
    )]);

    assert_eq!(
        logged(&sub, None),
        vec!["panes at load 0".to_string()],
        "a load was handed a session's geometry, or its ops were filed under one"
    );
    assert!(
        logged(&sub, Some(one())).is_empty(),
        "an op raised before any session existed was filed under a session"
    );
}
```

Note that `built` asserts `run_init()` is empty, which is where the load runs, so the op is
already buffered by the time the test reads it.

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux-lua --test subsystem a_config_load_runs_with_no_session`
Expected: PASS without any production change. If it fails, Task 3 bound a session somewhere it
should not have.

- [ ] **Step 3: Commit**

```bash
git add crates/neotmux-lua/tests/subsystem.rs
git commit -m "a load belongs to no session, and says so"
```

---

### Task 6: The spec says what the stages are

**Files:**
- Modify: `docs/superpowers/specs/2026-09-23-sessions-in-the-server-design.md`

- [ ] **Step 1: Amend §11**

Record, in place rather than by rewriting: the stages are reordered so that the socket
consolidation lands with N sessions, because `tests/sessions.rs` runs two names at once and a
one-entry map behind one socket cannot. Mark this plan's stage done.

- [ ] **Step 2: Amend §6**

It lists six pieces of state as needing to become per-session. Two do. Say which four do not
and why — the reservation and float APIs were already built as declarations, which is the same
rule §7 states from the other side.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-09-23-sessions-in-the-server-design.md
git commit -m "the spec counts the work again"
```

---

## What this plan does not do

- One socket, the CLI, and more than one session in the map. That is the next plan, and it is
  the one with observable behaviour in it.
- `neotmux.session.list` / `current` / `with` / `switch` / `create` / `kill`. The plan after.
- Per-session options. Still a non-goal.

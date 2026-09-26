# neotmux — a freeze is a hold, not a flag

Date: 2026-09-26
Status: proposed
Scope: #39. `neotmux.pane.freeze` returns a handle; a pane is frozen while anyone holds one;
a freeze taken through a group is released when the group is emptied.

## 1. Goal and non-goals

### Goal

The host's freeze is one `Option<Frozen>` per pane. Two callers freezing the same pane share
it, and the first to unfreeze thaws it for both. Today `plugin.copy` and `neotmux.hints` avoid
that by convention: each asks `pane.frozen` before freezing, remembers whether it was the one
that froze, and thaws only what it froze. Every new user of the freeze has to learn the same
convention, and one that gets it wrong thaws a pane out from under somebody else.

After this:

- Two holders of the same pane are independent: one releasing does not move the pane; the
  pane moves again when the last hold is released.
- A hold can only release itself. Releasing twice is harmless.
- A hold taken through `g.pane.freeze` is released when its group is emptied — named again,
  or dropped by a reload — so a reload cannot leave a pane frozen with nobody left who knows.
- `plugin.copy` and `neotmux.hints` no longer ask `pane.frozen` before freezing.

### Non-goals

- A different snapshot per holder. Holds share the snapshot the first one took, as a second
  `freeze` shares it today: re-taking the view would move a selection out from under the
  holder that anchored one.
- Releasing a hold when its Lua handle is garbage collected. A caller that drops the handle
  by accident would see the pane thaw at an arbitrary later GC.
- Sweeping ungrouped holds when a source fails to load or a reload runs. As with a timer armed
  outside a group, a hold from plain `neotmux.pane.freeze` is the caller's to release.
- A compatibility `pane.unfreeze(id)`. It is removed.

## 2. Host

### `TermState` (`neotmux-pty`)

Unchanged. `freeze`/`unfreeze`/`is_frozen` stay the on/off switch for the held snapshot.
Counting holds is not the terminal's business.

### `Hold` (`neotmux-proto::screen`)

```rust
pub struct Hold { pub pane: PaneId, id: u64 }
```

`Copy`, `Eq`, `Ord`, `Debug`. It lives beside `PaneRead`, which names it, in its own file
(`screen/hold.rs`, one file = one type). The id is unique for the life of the process; the pane
is carried so a release finds its terminal without a search. Only the implementor makes one:
the constructor is `Hold::new(pane, id)`, documented as the implementor's.

### `PaneRead` and `PaneTerminals`

`freeze(pane) -> bool` and `unfreeze(pane) -> bool` are replaced by:

```rust
fn hold(&self, pane: PaneId) -> Option<Hold>;   // None: no such pane
fn release(&self, hold: Hold) -> bool;          // false: already released, or the pane is gone
fn is_frozen(&self, pane: PaneId) -> Option<bool>; // unchanged
```

Each terminal in `PaneTerminals` gains `holds: BTreeSet<u64>`, and `PaneTerminals` a counter
for the next id.

- `hold`: if the set is empty, `state.freeze()` and record `frozen_revision`; insert a fresh id.
- `release`: remove the id from its pane's set; `false` if it was not there. If the set is now
  empty, `state.unfreeze()` and clear `frozen_revision`.
- A pane that closes takes its set with it, so a release after that answers `false`.

The wire is untouched; `PROTOCOL_VERSION` does not change.

## 3. Lua ABI

```lua
local h = neotmux.pane.freeze(id)   -- a handle, or nil if there is no such pane
h.pane                              -- the id it holds
h:release()                         -- true the first time; false after, or once the pane is gone
neotmux.pane.frozen(id)             -- true while anyone holds it, false, nil for no such pane

local g = neotmux.group('mine')
local h = g.pane.freeze(id)         -- the same, and released when `g` is emptied
```

- `neotmux.pane.unfreeze` is removed.
- `g.pane` is added to the group's nested tables (`("pane", &["freeze"])` in `group.rs`).
  The wrapped call notes `Held::Freeze(Hold)`; `Sweep::empty` releases it through the
  `PaneRead` it is given. `Sweep` gains the `Rc<dyn PaneRead>` for that.
- A reload empties every group before the config runs, so every grouped hold is released
  before the new code can take its own.

## 4. The shipped users

- `plugin.copy`: the `not frozen and freeze` dance and `state.froze` go; entering takes
  `g.pane.freeze(pane)` and keeps the handle, `leave` releases it. `hints.cancel()` on entry
  stays — it is there for the keyboard as much as for the freeze.
- `neotmux.hints`: the per-pane `already` check goes; `open` takes one `g.pane.freeze` per pane
  it labelled and keeps the handles, `close` releases them.
- Both keep their `unload` handlers, which also lower their modes. The release inside them is
  now redundant with the group's but harmless, since a release is idempotent.

## 5. Docs

`docs/plugins.md`: the pane API list, "Holding a pane still" (the "second freeze" bullet
becomes "holds share the first snapshot"; a section on handles and groups), the
`ctx:view` example, the hints paragraph that says "the host's freeze is a flag and not a
count", and the `unload` paragraph's freeze example. `CHANGELOG.md`: one entry.

## 6. Tests

- `neotmux-pty` `pane_terminals`: two holds on one pane — release one, still frozen and still
  on the first snapshot; release the other, live. A second release answers `false`. A release
  after the pane closed answers `false`. `hold` on a missing pane is `None`.
- `neotmux-lua/tests/freeze.rs`: the Lua shape — handle or `nil`, `h.pane`, `release` true
  then false; a `g.pane.freeze` hold released by naming the group again and by a reload; an
  ungrouped hold survives both.
- Existing tests move to the new API: `freeze.rs`, `hints.rs`, `copy.rs` (their helper
  snippets that call `pane.freeze(1)` keep the handle), `neotmux-server/src/shells.rs`.
- `hints.rs`'s "a pane frozen before hints opened stays frozen after" stays as the test of the
  property the convention used to buy.

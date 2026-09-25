# neotmux — neotmux-layout, one file per type

Date: 2026-09-25
Status: accepted (implemented 2026-09-25)
Scope: `neotmux-layout`'s `src/`, split so each public type has its own file. The first crate to
take yazi's "a file is a type" rule (deferred by `2026-09-25-crate-layout-design.md` §1); what it
settles is the convention the other crates follow. No behaviour changes.

## 1. The convention

1. **A public type, or a public free function that is a module's reason to exist, gets a file
   named after it** in `snake_case`: `PaneLayout` → `pane_layout.rs`, `assign` → `assign.rs`. A
   small type that only exists as one item's error or return value (`SplitRefused`, `Closed`) still
   gets its own file: the file name is where a reader looks for a type, and that must always hold.
2. **Private items live with the public item whose contract they implement**; failing that, with
   the one that owns the data they work on. `Tree`'s `Node`, `Split`, `Step` and the walks over
   them are in `tree.rs`; `usable`, `head` and `divide` are the arithmetic of `assign`'s contract,
   so they are in `assign.rs` even though `Tree::resize` calls them too. A private item another
   file calls becomes `pub(crate)` and is **imported by its file's path** —
   `use crate::assign::{divide, head, usable};` — so the owner is visible where it is used and two
   files' helpers can never meet in one flat namespace.
3. **`lib.rs` and `mod.rs` hold a `//!` doc, `mod` lines and `use` lines, nothing else.** A
   directory is a named `pub mod d;`. A file is flattened: `mod x; pub use x::*;` — a glob
   re-exports each item at its own visibility and never widens it, so this is right for files of
   public and `pub(crate)` items alike. The crate's `//!` doc in `lib.rs` is where its design notes
   go: a file module is private, so its own `//!` shows only under `--document-private-items`.
4. **A test goes in the file of the item it is about** — the item whose behaviour it asserts, not
   the items it uses to set up. Helpers shared by tests in two files go to a
   `#[cfg(test)] mod fixtures;` in the nearest `lib.rs` or `mod.rs` that contains both; a helper
   only one file's tests use stays in that file.
5. **A file whose items would collide with another's under flattening stays a named `pub mod x;`,
   and its items are not flattened.**
6. **A module whose public surface is free functions stays one named `pub mod`** — callers write
   `logging::init`, `procinfo::cwd`, `scrollback::lines`, and flattening would leave `init` or
   `lines` saying nothing. A public type in it moves out to its own file only if its name says what
   it is without the module in front (`ShutdownSignal` out of `signals`); one that reads only
   qualified (`tty::Any`, `sock::Paths`, `tty::Ready`) stays.
7. **Size is its own reason.** Rules 1 and 2 split by type, so one public entry point with a large
   private implementation stays one file. Where that file is past about 600 lines and its private
   part has separable concerns, the part moves to a private directory named after the item
   (`codec/` beside `codec.rs` becomes `codec/mod.rs` with its pieces). Each crate's spec decides
   where this applies.

## 2. neotmux-layout after

```
neotmux-layout/src/
  lib.rs              exports
  tree.rs             Tree; private Node, Split, Step and the tree walks
  assign.rs           assign; private usable, head, divide
  split_refused.rs    SplitRefused
  resize_refused.rs   ResizeRefused
  pane_layout.rs      PaneLayout; private applied, showable
  windows.rs          Windows; private Window
  surface.rs          Surface
  closed.rs           Closed
  shown.rs            Shown
  pane_ids.rs         PaneIds
  fixtures.rs         #[cfg(test)] helpers shared by tree.rs's and assign.rs's tests
```

The module doc of `layout.rs` goes to `tree.rs` (what a tree is) and `assign.rs` (why geometry is
one function — the seam issue #27 needs). The docs of `panes.rs` and `windows.rs` go to the file of
the type they describe.

### Tests

`layout.rs`'s 25 tests divide by subject:

- **`assign.rs`** (10) — the arithmetic of a given tree's rects: a lone pane, halving each way, the
  odd cell, gutters, the origin, exact cover (fixed and generated), and the generator's own range
  test.
- **`tree.rs`** (15) — what a split, a close and a resize do to the tree: refusals, the sibling that
  inherits, focus after close, the order `panes()` gives, and every resize test, including that a
  resized tree still tiles.

`panes.rs`'s and `windows.rs`'s tests stay with `PaneLayout` and `Windows`.

### Paths outside the crate

`neotmux-server` names four: `neotmux_layout::panes::PaneLayout` → `neotmux_layout::PaneLayout`,
and `neotmux_layout::windows::{PaneIds, Shown, Surface, Windows}` → `neotmux_layout::{…}`.

## 3. Evidence

`make check` passes with the same test count as before (1249), `neotmux-layout`'s own count equal
to its count before (64), and `cargo doc --workspace --no-deps --document-private-items` with no
warnings.

## 4. Later crates

- **neotmux-sys** (2026-09-25): `link.rs` is one type and is flattened (`neotmux_sys::Link`);
  `signals` keeps `HANGUP` and `die_of` and gives `ShutdownSignal` and `WindowChange` a file each;
  `logging`, `procinfo`, `sock` and `tty` stay named modules under rule 6.
- **neotmux-client** (2026-09-25): its one public type, `Decoder`, has `decoder.rs` (was
  `decode.rs`) and is flattened (`neotmux_client::Decoder`); `ask`, `attach`, `draw`, `overlay`,
  `start`, `osc52` and `client` are modules of functions whose types are private, and stay.
- **neotmux-pty** (2026-09-25): `child/` is `pty_handle.rs` (`PtyHandle`, and the
  `CommandBuilder` re-export its `spawn` takes), `pty_event.rs`, `wakeup.rs` (`wakeup()` and
  `Wakeup`) and `waker.rs`; `parse/query.rs` is `queries.rs` (`Queries`); `screen/state.rs` is
  `term_state.rs`; `terminals.rs` is `pane_terminals.rs`, flattened (`neotmux_pty::PaneTerminals`).
  `scrollback`, `evictions`, `lines` and `wrap` stay named modules under rule 6.

# neotmux — neotmux-layout, one file per type

Date: 2026-09-25
Status: proposed
Scope: `neotmux-layout`'s `src/`, split so each public type has its own file. The first crate to
take yazi's "a file is a type" rule (deferred by `2026-09-25-crate-layout-design.md` §1); what it
settles is the convention the other crates follow. No behaviour changes.

## 1. The convention

1. **A public type, or a public free function that is a module's reason to exist, gets a file
   named after it** in `snake_case`: `PaneLayout` → `pane_layout.rs`, `assign` → `assign.rs`.
2. **Private types and private helpers live with the public item they serve.** `Tree`'s `Node`,
   `Split`, `Step` and the walks over them (`insert`, `prune`, `descend`, …) are `Tree`'s
   implementation, so they are in `tree.rs`. A private item used by two public items' files goes
   to the one that owns the data it works on, and becomes `pub(crate)` only if the other file
   calls it.
3. **`lib.rs` and `mod.rs` export and nothing else.** Every file at a crate's root is flattened
   too (`mod x; pub use x::*;`), so a path is crate then item: `neotmux_layout::PaneLayout`. A file
   whose items were private stays `pub(crate) use`.
4. **A test goes in the file of the item it is about.** Where tests in two files share helpers,
   those helpers move to one `#[cfg(test)] mod fixtures;` at the crate root; a helper only one
   file's tests use stays in that file.
5. **Names that collide** under flattening are resolved as before: that file stays a named
   `pub mod`.

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

- **`assign.rs`** — the arithmetic of a given tree's rects: a lone pane, halving each way, the odd
  cell, gutters, the origin, the order panes come out in, exact cover (fixed and generated), and the
  generator's own range test.
- **`tree.rs`** — what a split, a close and a resize do to the tree: refusals, the sibling that
  inherits, focus after close, and every resize test, including that a resized tree still tiles.

`panes.rs`'s and `windows.rs`'s tests stay with `PaneLayout` and `Windows`.

### Paths outside the crate

`neotmux-server` names four: `neotmux_layout::panes::PaneLayout` → `neotmux_layout::PaneLayout`,
and `neotmux_layout::windows::{PaneIds, Shown, Surface, Windows}` → `neotmux_layout::{…}`.

## 3. Evidence

`make check` passes with the same test count as before (1249), `neotmux-layout`'s own count equal
to its count before (64), and `cargo doc --workspace --no-deps --document-private-items` with no
warnings.

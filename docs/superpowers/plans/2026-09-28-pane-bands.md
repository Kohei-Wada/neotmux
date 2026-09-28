# Pane bands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `g.reserve_pane('left', cells)` takes columns off the left of every pane, and `plugin.stamps` draws its times there instead of over the pane.

**Architecture:** The layout keeps cutting **outer** rects with `assign()`, and one function in `PaneLayout` (`inset`) derives each pane's **content** rect from it. Every existing reader of a pane's rect (pty size, grid placement, `ctx:pane_rect`, `neotmux.pane.rect`) gets the content rect. The outer rects travel to Lua in `Geometry.outer`, and Lua cuts each source's band out of them. The band's total width travels host-wards in `Tock.pane_band`, the way `reserved` and `gutter` already do.

**Tech Stack:** Rust (workspace crates `neotmux-layout`, `neotmux-proto`, `neotmux-lua`, `neotmux-server`), mlua, Lua runtime under `runtime/lua/`.

**Spec:** `docs/superpowers/specs/2026-09-28-pane-bands-design.md`

## Global Constraints

- Work in the worktree `.worktrees/pane-bands` (branch `pane-bands`). Never touch the main checkout.
- Only `'left'` is accepted as an edge. Any other edge raises.
- The cap is 16 columns, summed over every source (`MAX_PANE_BAND_COLS = 16`).
- One pane band per source. A second one answers `nil, message` until the first is removed.
- A pane whose outer width is less than `minimum.cols + pane_band` gets no band this frame. The decision is per pane.
- Split and resize are refused against `minimum.cols + pane_band`.
- The stamps band is 10 columns: `HH:MM:SS`, a space, then `│` in the dim colour. It has no background fill. A stall colours the time's **text**.
- No comments unless the why is non-obvious, and then one line. Match the surrounding prose-style doc comments.
- One file = one type in every crate (`PaneBands` gets its own file).
- `make check` (fmt, stylua/luacheck, clippy `-D warnings`, `cargo test --workspace`) must pass at the end.
- Commit messages follow the repo's style: lowercase, a sentence saying what is now true. End each with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_019EYxiTuCeWDUwaWAg81FbD
  ```

## Review Focus

1. **Toggling the band while a pane is zoomed.** The zoomed pane's pty narrows and widens. When unzoomed, every pane has the right width. Covered by a layout test in Task 1.
2. **A terminal shrunk until one pane is too narrow.** That pane keeps its whole outer rect, the others keep their bands, and `ctx:pane_band` is `nil` for the narrow one only. Covered in Task 1 (layout) and Task 2 (`band()` answers `None` when `content.w >= outer.w`).
3. **A pane split while a band is held.** The new pty is spawned at the content width, not the outer width. `pane_size()` reads `Windows::rect`, which becomes content. Covered by a headless test in Task 3.
4. **A band added inside a tick.** It must not be drawn over the pane's content that same frame, before the host has narrowed the pane. `band()` refuses a band that would reach past `content.x`. Covered in Task 2.
5. **Borders beside a banded pane.** The horizontal rule under a pane runs its full outer width, starting at the band's left edge, with no gap under the band. Covered in Task 4.

---

### Task 1: Outer and content rects in the layout

**Files:**
- Modify: `neotmux-layout/src/surface.rs`
- Modify: `neotmux-layout/src/pane_layout.rs` (struct, `new`, `resurface`, `reassign`, `split`, `resize`, `panes`, new `outer_panes`, new `inset`, tests)
- Modify: `neotmux-layout/src/windows.rs` (`push`, `resurface`, test helper `surface()`)
- Modify: `neotmux-server/src/session.rs` (the two `Surface { .. }` literals get `pane_band: 0` for now; Task 3 wires the real value)

**Interfaces:**
- Produces:
  - `Surface { size, reserved, gutter, pane_band: u16, minimum }`
  - `PaneLayout::new(full: Size, reserved: Reserved, gutter: u16, pane_band: u16, first: PaneId, minimum: Size)`
  - `PaneLayout::resurface(&mut self, full: Size, reserved: Reserved, gutter: u16, pane_band: u16)`
  - `PaneLayout::panes() -> Vec<(PaneId, PRect)>`, which now answers **content** rects
  - `PaneLayout::outer_panes() -> Vec<(PaneId, PRect)>` (new), which answers **outer** rects: the zoomed pane alone over `area()` while one is zoomed
  - `PaneLayout::rects` and `PaneLayout::shown`, which now hold **content** rects
  - `Windows::rect(pane)`, which now answers the content rect

- [ ] **Step 1: Write the failing tests** in the `tests` module of `neotmux-layout/src/pane_layout.rs`. Add them after `a_zoom_survives_the_surface_changing`:

```rust
    fn banded(rows: u16, cols: u16, band: u16) -> PaneLayout {
        PaneLayout::new(
            Size { rows, cols },
            Reserved::default(),
            0,
            band,
            pane(1),
            Size { rows: 2, cols: 2 },
        )
    }

    /// The band is taken off what the pane's shell gets, and not off where the pane sits: the
    /// outer rect is what the tree cut, and the content rect is the outer one less the band.
    #[test]
    fn a_pane_band_takes_its_columns_off_the_content_and_not_the_outer_rect() {
        let layout = banded(24, 80, 10);
        assert_eq!(layout.outer_panes(), vec![(pane(1), PRect::new(0, 0, 80, 24))]);
        assert_eq!(layout.panes(), vec![(pane(1), PRect::new(10, 0, 70, 24))]);
        assert_eq!(layout.rects[&pane(1)], PRect::new(10, 0, 70, 24));
        assert_eq!(
            layout.showable().collect::<Vec<_>>(),
            vec![(pane(1), PRect::new(10, 0, 70, 24))]
        );
    }

    #[test]
    fn every_pane_of_a_split_gets_its_own_band() {
        let mut layout = banded(24, 80, 10);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        assert_eq!(
            layout.outer_panes(),
            vec![
                (pane(1), PRect::new(0, 0, 40, 24)),
                (pane(2), PRect::new(40, 0, 40, 24)),
            ]
        );
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(10, 0, 30, 24)),
                (pane(2), PRect::new(50, 0, 30, 24)),
            ]
        );
    }

    /// One pane the terminal has shrunk below the minimum plus the band loses its band for the
    /// frame; the panes that still have room keep theirs.
    #[test]
    fn a_pane_too_narrow_for_its_band_keeps_its_outer_rect_and_the_others_keep_theirs() {
        let mut layout = banded(24, 80, 10);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        layout.split(SplitDir::Vertical, pane(3)).expect("fits");
        layout.resurface(Size { rows: 24, cols: 40 }, Reserved::default(), 0, 10);
        assert_eq!(
            layout.outer_panes(),
            vec![
                (pane(1), PRect::new(0, 0, 20, 24)),
                (pane(2), PRect::new(20, 0, 10, 24)),
                (pane(3), PRect::new(30, 0, 10, 24)),
            ]
        );
        assert_eq!(
            layout.panes(),
            vec![
                (pane(1), PRect::new(10, 0, 10, 24)),
                (pane(2), PRect::new(20, 0, 10, 24)),
                (pane(3), PRect::new(30, 0, 10, 24)),
            ]
        );
    }

    #[test]
    fn a_split_that_would_leave_a_pane_no_room_for_its_band_is_refused() {
        let mut with_band = banded(24, 20, 10);
        assert!(with_band.split(SplitDir::Vertical, pane(2)).is_err());
        assert_eq!(with_band.panes().len(), 1);

        let mut plain = layout(24, 20);
        assert!(
            plain.split(SplitDir::Vertical, pane(2)).is_ok(),
            "the same split fits with no band"
        );
    }

    #[test]
    fn a_zoomed_pane_has_its_band_over_the_whole_area() {
        let mut layout = banded(24, 80, 10);
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        layout.toggle_zoom();
        assert_eq!(layout.outer_panes(), vec![(pane(2), layout.area())]);
        assert_eq!(layout.panes(), vec![(pane(2), PRect::new(10, 0, 70, 24))]);

        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0, 0);
        assert_eq!(layout.panes(), vec![(pane(2), layout.area())]);
        layout.toggle_zoom();
        assert_eq!(layout.panes(), layout.outer_panes());
    }
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p neotmux-layout`
Expected: compile errors. `PaneLayout::new` takes 5 arguments, not 6, and `outer_panes` does not exist.

- [ ] **Step 3: Implement.**

In `neotmux-layout/src/surface.rs`, add the field after `gutter` and extend the doc comment's list:

```rust
pub struct Surface {
    pub size: Size,
    pub reserved: Reserved,
    pub gutter: u16,
    /// The columns every pane gives up down its left edge, summed over every source.
    pub pane_band: u16,
    pub minimum: Size,
}
```

In `neotmux-layout/src/pane_layout.rs`:

Add the field to `PaneLayout`, after `gutter`:

```rust
    /// The columns taken off the left of every pane's content, which the pane's outer rect
    /// still covers. A pane too narrow to spare them keeps its outer rect as its content.
    pane_band: u16,
```

Add this function after `showable`:

```rust
/// The one place a pane band costs a pane anything: its content is its outer rect less the
/// band, or the whole outer rect for a pane too narrow to spare it. Per pane, unlike
/// `applied()`, so that one small pane does not take every other pane's band away with it.
fn inset(outer: PRect, band: u16, minimum: Size) -> PRect {
    if band == 0 || outer.w < minimum.cols.saturating_add(band) {
        return outer;
    }
    PRect::new(outer.x + band, outer.y, outer.w - band, outer.h)
}
```

Change `new` and `resurface`:

```rust
    pub fn new(
        full: Size,
        reserved: Reserved,
        gutter: u16,
        pane_band: u16,
        first: PaneId,
        minimum: Size,
    ) -> Self {
        let mut layout = Self {
            content: Size::default(),
            rects: PaneRects::new(),
            applied: Reserved::default(),
            gutter: 0,
            pane_band: 0,
            minimum,
            tree: Tree::new(first),
            order: Vec::new(),
            focus: Some(first),
            zoomed: None,
            shown: PaneRects::new(),
        };
        layout.resurface(full, reserved, gutter, pane_band);
        layout
    }

    /// The surface changed, whether from a terminal resize, a band being reserved or
    /// released, the gutter between panes changing, or the pane band changing.
    pub fn resurface(&mut self, full: Size, reserved: Reserved, gutter: u16, pane_band: u16) {
        self.applied = applied(full, reserved, self.minimum);
        self.content = Size {
            rows: full.rows - self.applied.rows(),
            cols: full.cols - self.applied.cols(),
        };
        self.gutter = gutter;
        self.pane_band = pane_band;
        self.reassign();
    }
```

Change `reassign` so `rects` holds content. `order` stays outer:

```rust
    fn reassign(&mut self) {
        self.order = assign(&self.tree, self.area(), self.gutter);
        self.rects = self
            .order
            .iter()
            .map(|(id, outer)| (*id, inset(*outer, self.pane_band, self.minimum)))
            .collect();
        self.reshow();
    }
```

Add a helper and use it in `split` and `resize` in place of `self.minimum`:

```rust
    /// The smallest a pane may be cut to: the minimum its shell needs, plus the band it gives
    /// up, so a split cannot leave a pane that is only a band.
    fn cut_minimum(&self) -> Size {
        Size {
            rows: self.minimum.rows,
            cols: self.minimum.cols.saturating_add(self.pane_band),
        }
    }
```

```rust
        self.tree
            .split(target, dir, new, surface, self.cut_minimum(), self.gutter)?;
```

```rust
        self.tree
            .resize(target, dir, cells, surface, self.cut_minimum(), self.gutter)?;
```

Replace `panes()` with the pair:

```rust
    /// Every pane being shown and its outer rect — what the tree cut, bands and all — left to
    /// right and top to bottom: the zoomed pane alone, over the whole area, while one is.
    pub fn outer_panes(&self) -> Vec<(PaneId, PRect)> {
        match self.zoomed {
            Some(zoomed) => vec![(zoomed, self.area())],
            None => self.order.clone(),
        }
    }

    /// The same panes with their content rects, as `begin_frame` wants them: what each pane's
    /// shell is sized to and where its grid is drawn.
    pub fn panes(&self) -> Vec<(PaneId, PRect)> {
        self.outer_panes()
            .into_iter()
            .map(|(id, outer)| (id, inset(outer, self.pane_band, self.minimum)))
            .collect()
    }
```

Every existing `PaneLayout::new(full, reserved, gutter, first, minimum)` call in this file's tests becomes `PaneLayout::new(full, reserved, gutter, 0, first, minimum)`. Every `layout.resurface(size, reserved, gutter)` gets a trailing `, 0`. The compiler lists each one.

In `neotmux-layout/src/windows.rs`:

```rust
            layout: PaneLayout::new(
                surface.size,
                surface.reserved,
                surface.gutter,
                surface.pane_band,
                pane,
                surface.minimum,
            ),
```

```rust
    pub fn resurface(&mut self, surface: Surface) {
        self.entries[self.current].layout.resurface(
            surface.size,
            surface.reserved,
            surface.gutter,
            surface.pane_band,
        );
    }
```

In the test helper `surface()`, add `pane_band: 0,`.

In `neotmux-server/src/session.rs`, add `pane_band: 0,` to the `Surface { .. }` literal in `Session::new` (around line 102) and to the one in the test `the_first_pane_is_the_whole_surface` (around line 914).

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p neotmux-layout && cargo build --workspace --all-targets`
Expected: every layout test passes and the workspace builds.

- [ ] **Step 5: Commit**

```bash
git add neotmux-layout neotmux-server/src/session.rs
git commit -m "a pane's outer rect is what the tree cuts and its content is that less the pane band"
```

---

### Task 2: `reserve_pane`, `ctx:pane_band` and `ctx:pane_outer` in the Lua layer

**Files:**
- Modify: `neotmux-proto/src/geom/geometry.rs` (the `outer` field)
- Create: `neotmux-lua/src/draw/pane_bands.rs` (the `PaneBands` store and its unit tests)
- Modify: `neotmux-lua/src/draw/mod.rs`
- Create: `neotmux-lua/src/api/reserve_pane.rs`
- Modify: `neotmux-lua/src/api/mod.rs`, `neotmux-lua/src/api/install.rs`, `neotmux-lua/src/api/group.rs`, `neotmux-lua/src/api/sweep.rs`, `neotmux-lua/src/plugin/held.rs`, `neotmux-lua/src/api/ctx.rs`, `neotmux-lua/src/lua_subsystem.rs`
- Modify: `neotmux-server/src/session.rs` (`geometry()` fills `outer`)
- Test: `neotmux-lua/tests/reserve_pane.rs` (new)

**Interfaces:**
- Consumes: `PaneLayout::outer_panes()` from Task 1.
- Produces:
  - `Geometry.outer: Vec<(PaneId, Rect)>`. It is empty in a geometry built with `..Default::default()`.
  - `neotmux_lua::draw::PaneBands` with `new`, `add(owner, cells) -> Result<ReservationId, String>`, `resize(id, cells) -> Result<(), String>`, `remove(id) -> bool`, `remove_all_of(owner) -> usize`, `total() -> u16`, `band(owner, outer: Rect, content: Rect) -> Option<Rect>`
  - `neotmux_lua::draw::MAX_PANE_BAND_COLS: u16 = 16`
  - `LuaSubsystem::pane_band(&self) -> u16` (the total)
  - Lua: `neotmux.reserve_pane(edge, cells) -> handle | nil, message`, the same through `neotmux.group(name).reserve_pane`, `handle:remove()`, `handle:resize_to(cells) -> true | nil, message`
  - Lua: `ctx:pane_band(pane) -> {x,y,w,h} | nil` and `ctx:pane_outer(pane) -> {x,y,w,h} | nil`

- [ ] **Step 1: Write the failing integration test** `neotmux-lua/tests/reserve_pane.rs`:

```rust
//! `neotmux.reserve_pane` across the ABI: what a source is granted, what it is refused, and
//! which columns of a pane it draws in.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size};

mod support;

const SURFACE: Size = Size { rows: 24, cols: 80 };

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

/// What the host publishes for one pane over the whole surface, `band` columns of it given up
/// on the left — or, with `band` 0, a pane that did not get its bands this frame.
fn frame(sub: &LuaSubsystem, band: u16) {
    let pane = PaneId::from(1);
    let outer = Rect::new(0, 0, SURFACE.cols, SURFACE.rows);
    let content = Rect::new(band, 0, SURFACE.cols - band, SURFACE.rows);
    sub.begin_frame(
        support::one(),
        Epoch::new(SURFACE, vec![pane], Some(pane)),
        Geometry {
            area: outer,
            panes: vec![(pane, content)],
            outer: vec![(pane, outer)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());
}

fn texts(sub: &LuaSubsystem) -> Vec<String> {
    sub.frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A plugin that takes `cells` of every pane and reports where its band in pane 1 landed.
fn reporter(name: &str, cells: u16) -> String {
    format!(
        r#"
        neotmux.reserve_pane('left', {cells})
        neotmux.on_tick(function(ctx)
          local band = ctx:pane_band(1)
          ctx:text(nil, 0, 0, '{name}=' .. (band and (band.x .. '+' .. band.w .. 'x' .. band.h) or 'nil'))
        end)
        return {{}}
        "#
    )
}

#[test]
fn two_sources_get_their_bands_side_by_side_from_the_pane_edge_in() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", &reporter("a", 3)),
        ("lua/plugin/b.lua", &reporter("b", 4)),
    ]);
    assert_eq!(sub.pane_band(), 7);
    frame(&sub, 7);
    assert_eq!(texts(&sub), vec!["a=0+3x24".to_string(), "b=3+4x24".to_string()]);
}

#[test]
fn a_pane_that_did_not_get_its_bands_has_none_to_draw_in() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a')"),
        ("lua/plugin/a.lua", &reporter("a", 3)),
    ]);
    frame(&sub, 0);
    assert_eq!(texts(&sub), vec!["a=nil".to_string()]);
}

/// A band asked for inside a tick is not drawn until the host has narrowed the pane for it.
#[test]
fn a_band_the_pane_has_not_made_room_for_yet_is_not_handed_out() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", &reporter("a", 3)),
        ("lua/plugin/b.lua", &reporter("b", 4)),
    ]);
    frame(&sub, 3);
    assert_eq!(texts(&sub), vec!["a=0+3x24".to_string(), "b=nil".to_string()]);
}

#[test]
fn the_outer_rect_is_the_pane_with_its_bands_and_falls_back_to_the_content() {
    let (_dir, sub) = built(&[(
        "init.lua",
        r#"
        neotmux.on_tick(function(ctx)
          local r = ctx:pane_outer(1)
          ctx:text(nil, 0, 0, r.x .. '+' .. r.w)
          ctx:text(nil, 0, 1, tostring(ctx:pane_outer(99)))
        end)
        "#,
    )]);
    frame(&sub, 5);
    assert_eq!(texts(&sub), vec!["0+80".to_string(), "nil".to_string()]);

    let pane = PaneId::from(1);
    let content = Rect::new(5, 0, 75, 24);
    sub.begin_frame(
        support::one(),
        Epoch::new(SURFACE, vec![pane], Some(pane)),
        Geometry {
            area: content,
            panes: vec![(pane, content)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());
    assert_eq!(texts(&sub), vec!["5+75".to_string(), "nil".to_string()]);
}

#[test]
fn a_second_band_from_one_source_and_one_past_the_cap_are_refused_with_a_message() {
    let (_dir, sub) = built(&[(
        "init.lua",
        r#"
        local first = neotmux.reserve_pane('left', 10)
        local second, why = neotmux.reserve_pane('left', 1)
        assert(first and second == nil and why:find('already holds'), why)
        assert(first:resize_to(16))
        assert(not pcall(first.resize_to, first, 17), 'a count past the cap raises')
        "#,
    )]);
    assert_eq!(sub.pane_band(), 16);

    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", "assert(neotmux.reserve_pane('left', 10)) return {}"),
        (
            "lua/plugin/b.lua",
            "local h, why = neotmux.reserve_pane('left', 7)\n\
             assert(h == nil and why:find('will not fit'), why)\n\
             return {}",
        ),
    ]);
    assert_eq!(sub.pane_band(), 10);
}

#[test]
fn an_edge_other_than_left_and_a_count_out_of_range_raise() {
    for body in [
        "neotmux.reserve_pane('top', 1)",
        "neotmux.reserve_pane('left', 0)",
        "neotmux.reserve_pane('left', 17)",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("init.lua"), body).unwrap();
        let sub = LuaSubsystem::build(
            Layers::from_paths(vec![dir.path().to_path_buf()]),
            support::panes(),
        )
        .unwrap();
        assert!(!sub.run_init().is_empty(), "{body} did not raise");
        assert_eq!(sub.pane_band(), 0);
    }
}

#[test]
fn removing_the_handle_or_emptying_its_group_gives_the_columns_back() {
    let (_dir, sub) = built(&[(
        "init.lua",
        r#"
        neotmux.reserve_pane('left', 4):remove()
        local g = neotmux.group('g')
        g.reserve_pane('left', 6)
        neotmux.group('g')
        "#,
    )]);
    assert_eq!(sub.pane_band(), 0);
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p neotmux-lua --test reserve_pane`
Expected: a compile error. `Geometry` has no field `outer`, and `pane_band` does not exist.

- [ ] **Step 3: Implement.**

`neotmux-proto/src/geom/geometry.rs`, after `panes`:

```rust
    /// Every shown pane's outer rect: its rect in `panes` with the pane bands put back. A pane
    /// whose outer rect is wider than its content got its bands this frame.
    pub outer: Vec<(PaneId, Rect)>,
```

`neotmux-server/src/session.rs`, in `geometry()`, after `panes: layout.panes(),`:

```rust
            outer: layout.outer_panes(),
```

Build once (`cargo build --workspace --all-targets`). Any `Geometry { .. }` literal without `..Default::default()` that the compiler names gets `outer: Vec::new(),`.

`neotmux-lua/src/draw/pane_bands.rs`:

```rust
use std::cell::RefCell;

use neotmux_proto::Rect;

use crate::draw::ReservationId;
use crate::plugin::SourceId;

/// The most columns every pane band together may take off a pane. A pane band is a gutter of
/// text down a pane's edge — a time, a line number — and past this it is not a gutter but the
/// pane.
pub const MAX_PANE_BAND_COLS: u16 = 16;

struct Entry {
    id: ReservationId,
    owner: SourceId,
    cells: u16,
}

#[derive(Default)]
struct Inner {
    next: u64,
    /// In grant order, which is load order: a pane's bands are filled from its left edge in.
    entries: Vec<Entry>,
}

impl Inner {
    fn taken(&self, except: Option<ReservationId>) -> u16 {
        self.entries
            .iter()
            .filter(|entry| Some(entry.id) != except)
            .map(|entry| entry.cells)
            .sum()
    }
}

/// Who has asked for columns down the left of every pane, and how many. `Reservations` one
/// level down: the host takes the total off every pane it can spare it from, and each source
/// is handed its own columns of what the host took.
#[derive(Default)]
pub struct PaneBands {
    inner: RefCell<Inner>,
}

impl PaneBands {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Err` carries the message handed back to the caller; nothing is changed in that case.
    pub fn add(&self, owner: SourceId, cells: u16) -> Result<ReservationId, String> {
        let mut inner = self.inner.borrow_mut();
        if inner.entries.iter().any(|entry| entry.owner == owner) {
            return Err(
                "already holds a pane band; remove it before asking for another".to_string(),
            );
        }
        let taken = inner.taken(None);
        if taken + cells > MAX_PANE_BAND_COLS {
            return Err(format!(
                "every pane has {taken} of its {MAX_PANE_BAND_COLS} band columns reserved, \
                 so {cells} more will not fit"
            ));
        }
        inner.next += 1;
        let id = ReservationId(inner.next);
        inner.entries.push(Entry { id, owner, cells });
        Ok(id)
    }

    /// A granted band takes a different number of columns from the next frame on, keeping its
    /// place in grant order.
    pub fn resize(&self, id: ReservationId, cells: u16) -> Result<(), String> {
        let mut inner = self.inner.borrow_mut();
        if !inner.entries.iter().any(|entry| entry.id == id) {
            return Err("the pane band has already been removed".to_string());
        }
        let taken = inner.taken(Some(id));
        if taken + cells > MAX_PANE_BAND_COLS {
            return Err(format!(
                "every pane has {taken} of its {MAX_PANE_BAND_COLS} band columns reserved by \
                 others, so {cells} will not fit"
            ));
        }
        for entry in inner.entries.iter_mut() {
            if entry.id == id {
                entry.cells = cells;
            }
        }
        Ok(())
    }

    pub fn remove(&self, id: ReservationId) -> bool {
        let mut inner = self.inner.borrow_mut();
        let before = inner.entries.len();
        inner.entries.retain(|entry| entry.id != id);
        before != inner.entries.len()
    }

    pub fn remove_all_of(&self, owner: SourceId) -> usize {
        let mut inner = self.inner.borrow_mut();
        let before = inner.entries.len();
        inner.entries.retain(|entry| entry.owner != owner);
        before - inner.entries.len()
    }

    /// The columns every pane is asked to give up, summed over every source.
    pub fn total(&self) -> u16 {
        self.inner.borrow().taken(None)
    }

    /// `owner`'s columns of a pane whose rect is `outer` with its bands and `content` without
    /// them, or `None` when `owner` holds none, the pane did not get its bands this frame, or
    /// the band would reach past what the host took — one granted since the host last laid
    /// out, which it has not made room for yet.
    pub fn band(&self, owner: SourceId, outer: Rect, content: Rect) -> Option<Rect> {
        if content.w >= outer.w {
            return None;
        }
        let inner = self.inner.borrow();
        let mut x = outer.x;
        for entry in &inner.entries {
            let end = x.saturating_add(entry.cells);
            if entry.owner == owner {
                return (end <= content.x).then(|| Rect::new(x, outer.y, entry.cells, outer.h));
            }
            x = end;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Sources;

    fn sources() -> (Sources, SourceId, SourceId) {
        let sources = Sources::new();
        let a = sources.intern("@a.lua");
        let b = sources.intern("@b.lua");
        (sources, a, b)
    }

    const OUTER: Rect = Rect {
        x: 20,
        y: 1,
        w: 40,
        h: 10,
    };

    #[test]
    fn bands_are_handed_out_from_the_pane_edge_in_grant_order() {
        let (_s, a, b) = sources();
        let bands = PaneBands::new();
        bands.add(a, 3).expect("granted");
        bands.add(b, 4).expect("granted");
        assert_eq!(bands.total(), 7);
        let content = Rect::new(27, 1, 33, 10);
        assert_eq!(bands.band(a, OUTER, content), Some(Rect::new(20, 1, 3, 10)));
        assert_eq!(bands.band(b, OUTER, content), Some(Rect::new(23, 1, 4, 10)));
    }

    #[test]
    fn a_pane_without_its_bands_hands_none_out() {
        let (_s, a, _b) = sources();
        let bands = PaneBands::new();
        bands.add(a, 3).expect("granted");
        assert_eq!(bands.band(a, OUTER, OUTER), None);
    }

    #[test]
    fn the_cap_counts_every_source_and_resize_keeps_the_order() {
        let (_s, a, b) = sources();
        let bands = PaneBands::new();
        let first = bands.add(a, 10).expect("granted");
        assert!(bands.add(b, 7).is_err());
        bands.add(b, 6).expect("granted");
        assert!(bands.resize(first, 11).is_err());
        bands.resize(first, 2).expect("fits");
        let content = Rect::new(28, 1, 32, 10);
        assert_eq!(bands.band(b, OUTER, content), Some(Rect::new(22, 1, 6, 10)));
    }

    #[test]
    fn one_band_per_source() {
        let (_s, a, _b) = sources();
        let bands = PaneBands::new();
        let id = bands.add(a, 1).expect("granted");
        assert!(bands.add(a, 1).is_err());
        assert!(bands.remove(id));
        bands.add(a, 1).expect("granted after the first went");
    }
}
```

Check that `Rect` has public fields `x, y, w, h` so the `const OUTER` literal compiles. If it does not, build `OUTER` with `Rect::new` inside a `fn outer() -> Rect`.

`neotmux-lua/src/draw/mod.rs`: add `mod pane_bands;` and `pub use pane_bands::*;` in the alphabetical positions.

`neotmux-lua/src/plugin/held.rs`: add a variant after `Reservation`:

```rust
    PaneBand(ReservationId),
```

`neotmux-lua/src/api/sweep.rs`:
- Import `PaneBands` alongside `Reservations`.
- Add the field `pub(crate) pane_bands: Rc<PaneBands>,` after `reservations`.
- In `remove_all_of`, add `self.pane_bands.remove_all_of(id);` after the reservations line.
- In `empty`, add the arm:

```rust
                Held::PaneBand(id) => {
                    self.pane_bands.remove(id);
                }
```

`neotmux-lua/src/api/reserve_pane.rs`:

```rust
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::draw::{MAX_PANE_BAND_COLS, PaneBands, ReservationId};
use crate::plugin::{self, Groups, Held, Sources};

/// A count no pane could ever give is a typo rather than a lost race, so it raises.
fn check_cells(call: &str, cells: u16) -> mlua::Result<()> {
    if cells == 0 {
        return Err(mlua::Error::RuntimeError(format!(
            "{call} takes at least one column"
        )));
    }
    if cells > MAX_PANE_BAND_COLS {
        return Err(mlua::Error::RuntimeError(format!(
            "{call} takes at most {MAX_PANE_BAND_COLS} columns; a pane band is a gutter, not \
             a pane"
        )));
    }
    Ok(())
}

fn handle(lua: &Lua, bands: Rc<PaneBands>, id: ReservationId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    let remove = {
        let bands = bands.clone();
        lua.create_function(move |_, _self: Value| {
            bands.remove(id);
            Ok(())
        })?
    };
    handle.set("remove", remove)?;
    let resize_to = lua.create_function(move |lua, (_self, cells): (Value, u16)| {
        check_cells("handle:resize_to", cells)?;
        match bands.resize(id, cells) {
            Ok(()) => Ok((Value::Boolean(true), Value::Nil)),
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })?;
    handle.set("resize_to", resize_to)?;
    Ok(handle)
}

/// `neotmux.reserve_pane(edge, cells)`: columns down the edge of every pane, which that pane's
/// shell no longer gets. `neotmux.reserve` one level down, answering the same two ways.
pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    bands: Rc<PaneBands>,
    groups: Rc<Groups>,
) -> mlua::Result<Function> {
    lua.create_function(move |lua, (edge, cells): (String, u16)| {
        if edge != "left" {
            return Err(mlua::Error::RuntimeError(format!(
                "unknown edge '{edge}': neotmux.reserve_pane takes 'left'"
            )));
        }
        check_cells("neotmux.reserve_pane", cells)?;
        let owner = plugin::caller(lua, &sources);
        match bands.add(owner, cells) {
            Ok(id) => {
                groups.note(Held::PaneBand(id));
                Ok((Value::Table(handle(lua, bands.clone(), id)?), Value::Nil))
            }
            Err(message) => Ok((Value::Nil, Value::String(lua.create_string(&message)?))),
        }
    })
}
```

`neotmux-lua/src/api/mod.rs`: add `mod reserve_pane;` after `pub mod reserve;`.

`neotmux-lua/src/api/install.rs`:
- Add `pub(crate) pane_bands: Rc<PaneBands>,` to `Shared` after `reservations`, and import `PaneBands` where `Reservations` is imported.
- Add `pane_bands,` to the destructure.
- After the `reserve` block:

```rust
    {
        let reserve_pane =
            super::reserve_pane::install(lua, sources.clone(), pane_bands.clone(), groups.clone())?;
        neotmux.set("reserve_pane", reserve_pane)?;
    }
```

`neotmux-lua/src/api/group.rs`:

```rust
const TOP: [&str; 7] = [
    "on_tick", "on_event", "on_key", "on_write", "float", "reserve", "reserve_pane",
];
```

`neotmux-lua/src/api/ctx.rs`:
- Add fields after `band`:

```rust
    /// Every shown pane's outer rect, bands and all. Empty from a host that has none to give,
    /// and then a pane's outer rect is its content.
    outer: Vec<(PaneId, Rect)>,
    /// This source's own columns of each pane that got its bands this frame.
    pane_bands: BTreeMap<PaneId, Rect>,
```

- Extend `Ctx::new` with the arguments `outer: Vec<(PaneId, Rect)>` (after `panes`) and `pane_bands: BTreeMap<PaneId, Rect>` (after `band`), and store them.
- Add methods after `reserved`:

```rust
        methods.add_method("pane_band", |lua, this, pane: u32| {
            match this.pane_bands.get(&PaneId::from(pane)) {
                Some(band) => Ok(Value::Table(rect::to_table(lua, *band)?)),
                None => Ok(Value::Nil),
            }
        });

        methods.add_method("pane_outer", |lua, this, pane: u32| {
            let id = PaneId::from(pane);
            let found = this
                .outer
                .iter()
                .find(|(pane, _)| *pane == id)
                .map(|(_, rect)| *rect)
                .or_else(|| this.pane_rect(id));
            match found {
                Some(found) => Ok(Value::Table(rect::to_table(lua, found)?)),
                None => Ok(Value::Nil),
            }
        });
```

`neotmux-lua/src/lua_subsystem.rs`:
- Add the field `pane_bands: Rc<PaneBands>,` next to `reservations`, and import it.
- In `build`, add `let pane_bands = Rc::new(PaneBands::new());` after `reservations`, and pass `pane_bands: pane_bands.clone(),` into `api::Sweep { .. }` and `api::Shared { .. }`, and `pane_bands,` into `Self { .. }`.
- Build the ctx like this:

```rust
                let ctx = {
                    let geometry = self.geometry.borrow();
                    let pane_bands = geometry
                        .outer
                        .iter()
                        .filter_map(|(pane, outer)| {
                            let (_, content) = geometry.panes.iter().find(|(p, _)| p == pane)?;
                            self.pane_bands
                                .band(owner, *outer, *content)
                                .map(|band| (*pane, band))
                        })
                        .collect();
                    Ctx::new(
                        owner,
                        self.buffers.clone(),
                        geometry.area,
                        geometry.panes.clone(),
                        geometry.outer.clone(),
                        self.reservations.band(owner),
                        pane_bands,
                        self.floats.by_owner(owner),
                    )
                };
```

- After `reserved()`:

```rust
    /// The columns every source has asked every pane for, summed. Which panes can spare them
    /// is the host's judgement.
    pub fn pane_band(&self) -> u16 {
        self.pane_bands.total()
    }
```

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p neotmux-lua --test reserve_pane && cargo test -p neotmux-lua --lib pane_bands && cargo build --workspace --all-targets`
Expected: PASS and the workspace builds. If some test enumerates the `neotmux` table's keys or the group's fields (grep `"reserve"` under `neotmux-lua/tests` and `neotmux/tests`), add `reserve_pane` to its expected list.

- [ ] **Step 5: Commit**

```bash
git add neotmux-proto neotmux-lua neotmux-server/src/session.rs
git commit -m "neotmux.reserve_pane takes columns down every pane, and ctx:pane_band answers a source its own"
```

---

### Task 3: The host narrows the panes

**Files:**
- Modify: `neotmux-proto/src/wire/tock.rs`, `neotmux-server/src/setup.rs`, `neotmux-server/src/session.rs` (`new`, `adopt`, `apply`, new `pane_band_changed`), `neotmux-server/src/plugins.rs:156-162`, `neotmux-server/src/doorstep.rs:319-325`, `neotmux-server/src/sessions.rs:219-227`
- Test: `neotmux-server/tests/headless.rs`

**Interfaces:**
- Consumes: `LuaSubsystem::pane_band()` (Task 2), `Surface.pane_band` (Task 1).
- Produces: `Tock.pane_band: u16` and `Setup.pane_band: u16`.

- [ ] **Step 1: Write the failing tests** at the end of `neotmux-server/tests/headless.rs`. Also add `pane_band: 0,` to the `Setup { .. }` in `halves` and to the `Tock { .. }` in `two_applies_are_two_stops_on_the_way_to_two_frames`:

```rust
/// A pane band held by a plugin narrows the pane's shell by its columns and moves its grid
/// across, and giving it back puts both back.
#[test]
fn a_pane_band_narrows_the_shell_and_giving_it_back_widens_it() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[(
        "init.lua",
        "local held\n\
         neotmux.keymap.set('B', function()\n\
           if held then held:remove() held = nil\n\
           else held = neotmux.reserve_pane('left', 10) end\n\
           return true\n\
         end)\n",
    )]);
    key(&mut session, &plugins, Key::plain(KeyCode::Char('B')));
    assert!(
        step_until(&mut session, &mut plugins, |frame| {
            frame.panes.iter().all(|pane| {
                pane.rect.x == 10 && pane.grid.size.cols == SURFACE.cols - 10
            })
        }),
        "the pane was not narrowed"
    );
    key(&mut session, &plugins, Key::plain(KeyCode::Char('B')));
    assert!(
        step_until(&mut session, &mut plugins, |frame| {
            frame
                .panes
                .iter()
                .all(|pane| pane.rect.x == 0 && pane.grid.size.cols == SURFACE.cols)
        }),
        "the pane did not get its columns back"
    );
}

/// A pane made while a band is held starts at its content width, not at the width of the
/// rect the tree cut for it.
#[test]
fn a_pane_split_while_a_band_is_held_starts_at_its_content_width() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[(
        "init.lua",
        "neotmux.reserve_pane('left', 5)\n\
         neotmux.keymap.set('S', function() neotmux.pane.split('vertical') return true end)\n",
    )]);
    assert!(step_until(&mut session, &mut plugins, |frame| {
        frame.panes.iter().all(|pane| pane.rect.x == 5)
    }));
    key(&mut session, &plugins, Key::plain(KeyCode::Char('S')));
    assert!(
        step_until(&mut session, &mut plugins, |frame| {
            frame.panes.len() == 2
                && frame
                    .panes
                    .iter()
                    .all(|pane| pane.grid.size.cols == pane.rect.w && pane.rect.w == 15)
        }),
        "the new pane's shell is not its content's width"
    );
}
```

(40 columns split vertically gives two outer rects of 20 each, and each pane's content is 15.) Check the name of the split call against `docs/plugins.md` ("### Panes"). If it is `neotmux.pane.split('vertical')`, keep it. Otherwise use the documented spelling.

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p neotmux-server --test headless pane_band`
Expected: a compile error. `Setup` and `Tock` have no field `pane_band`.

- [ ] **Step 3: Implement.**

`neotmux-proto/src/wire/tock.rs`: add `pub pane_band: u16,` after `gutter`.

`neotmux-server/src/setup.rs`: add `pub pane_band: u16,` after `gutter`. Update the doc comment to "The other five are the Lua layer's", and add "the columns every pane gives up".

`neotmux-server/src/plugins.rs`, in the `Tock { .. }`: `pane_band: self.lua.pane_band(),`

`neotmux-server/src/doorstep.rs` and `neotmux-server/src/sessions.rs`, in the `Setup { .. }`: `pane_band: lua.pane_band(),`

`neotmux-server/src/session.rs`:
- In `Session::new`'s `Surface`, change the Task 1 placeholder to `pane_band: setup.pane_band,`.
- In `adopt`, add `self.surface.pane_band = setup.pane_band;` after the gutter line.
- After `gutter_changed`:

```rust
    /// Whether the columns every pane gives up just changed, which changes every pane's size.
    /// Sampled at the frame boundary for the same reason `reserved_changed` is.
    fn pane_band_changed(&mut self, pane_band: u16) -> bool {
        if pane_band == self.surface.pane_band {
            return false;
        }
        self.surface.pane_band = pane_band;
        true
    }
```

- In `apply`:

```rust
        let reserved_changed = self.reserved_changed(tock.reserved);
        let gutter_changed = self.gutter_changed(tock.gutter);
        let pane_band_changed = self.pane_band_changed(tock.pane_band);
        if reserved_changed || gutter_changed || pane_band_changed {
            self.apply_geometry()?;
        }
```

Build. The compiler names every other `Tock { .. }` and `Setup { .. }` literal, and each one gets `pane_band: 0,`.

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p neotmux-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add neotmux-proto neotmux-server
git commit -m "a pane band held in the Lua layer narrows every pane's shell, and giving it back widens them"
```

---

### Task 4: The borders and the prompt draw around the outer rect

**Files:**
- Modify: `runtime/lua/plugin/tmux/borders.lua:66,75`, `runtime/lua/neotmux/prompt.lua:100`
- Test: `neotmux-lua/tests/borders.rs` (new)

**Interfaces:**
- Consumes: `ctx:pane_outer(pane)` (Task 2).

- [ ] **Step 1: Write the failing test** `neotmux-lua/tests/borders.rs`:

```rust
//! `plugin.tmux.borders` beside panes that have given columns to a pane band.

mod support;

use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect};
use support::{Built, built, one};

/// Two panes, one above the other, over the whole 40x16 surface with a one-row gutter between
/// them, each with `band` columns given up on its left.
fn stacked(sub: &Built, band: u16) {
    let (top, bottom) = (PaneId::from(1), PaneId::from(2));
    let top_outer = Rect::new(0, 0, 40, 7);
    let bottom_outer = Rect::new(0, 8, 40, 8);
    let inset = |r: Rect| Rect::new(r.x + band, r.y, r.w - band, r.h);
    sub.begin_frame(
        one(),
        Epoch::new(support::SIZE, vec![top, bottom], Some(top)),
        Geometry {
            area: Rect::new(0, 0, 40, 16),
            panes: vec![(top, inset(top_outer)), (bottom, inset(bottom_outer))],
            outer: vec![(top, top_outer), (bottom, bottom_outer)],
            focus: Some(top),
            ..Default::default()
        },
    );
    sub.dispatch_tick(one());
}

#[test]
fn the_rule_under_a_banded_pane_runs_under_its_band_too() {
    let (_dir, sub) = built("require('plugin.tmux.borders')");
    stacked(&sub, 10);
    let starts: Vec<u16> = sub
        .frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { x, y: 7, .. } => Some(x),
            _ => None,
        })
        .collect();
    assert_eq!(
        starts.iter().min(),
        Some(&0),
        "the rule starts at the band, not at the content: {starts:?}"
    );
}
```

Check two things against `neotmux-lua/tests/support/mod.rs`. First, `support::built` must not stop `plugin.tmux.borders` loading when it is `require`d directly. It only flips `neotmux.defaults`, and a direct `require` ignores those, but confirm it. Second, confirm how `ctx:line` reaches `frame_output`, as `DrawCmd::Text` runs or another variant. Match whatever variant the rule is emitted as. The assertion is only about the smallest `x` on row 7.

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p neotmux-lua --test borders`
Expected: FAIL. The smallest `x` on row 7 is 10.

- [ ] **Step 3: Implement.** In `runtime/lua/plugin/tmux/borders.lua`, change both `local r = ctx:pane_rect(id)` lines to `local r = ctx:pane_outer(id)`. In `runtime/lua/neotmux/prompt.lua:100`, change `ctx:pane_rect(pane)` to `ctx:pane_outer(pane)`.

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p neotmux-lua`
Expected: PASS. The other suites have no outer rects, so they fall back to the content rect.

- [ ] **Step 5: Commit**

```bash
git add runtime/lua/plugin/tmux/borders.lua runtime/lua/neotmux/prompt.lua neotmux-lua/tests/borders.rs
git commit -m "the borders and the prompt go round a pane's outer rect, so a pane band leaves no gap in a rule"
```

---

### Task 5: `plugin.stamps` draws in its own band

**Files:**
- Modify: `runtime/lua/plugin/stamps.lua`
- Modify: `neotmux-lua/tests/stamps.rs`

**Interfaces:**
- Consumes: `g.reserve_pane`, `ctx:pane_band` (Task 2).
- Produces: `require('plugin.stamps')` keeps `toggle`, `shown`, `stall` and `colors`. `colors` becomes `{ time, rule, slow, stuck }`, because `gutter` goes. `width` becomes 10.

- [ ] **Step 1: Rewrite the tests.** Replace the helpers and assertions in `neotmux-lua/tests/stamps.rs`.
  - The band only exists once the host publishes an outer rect, so add a `banded(sub)` helper. It re-publishes the geometry with the pane's outer rect over the whole surface and its content 10 columns in, then ticks.
  - The gutter now finds times by `DrawCmd::Text` at `x == 0`, and the rule by a `│` at `x == 9`.
  - A stall is found by the time's `fg`, not by a `Rect`'s `bg`.

```rust
use neotmux_proto::{Color, DrawCmd, Epoch, Geometry, Rect};
use support::{Built, SIZE, built, feed, keys, one, pane};

/// What the host publishes once it has taken the band off the pane: the pane's outer rect over
/// the whole surface and its content ten columns in.
fn banded(sub: &Built) {
    let outer = Rect::new(0, 0, SIZE.cols, SIZE.rows);
    sub.begin_frame(
        one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: outer,
            panes: vec![(pane(), Rect::new(10, 0, SIZE.cols - 10, SIZE.rows))],
            outer: vec![(pane(), outer)],
            focus: Some(pane()),
            ..Default::default()
        },
    );
}

/// Each row's time, with the colour it is written in, and the rows the rule is drawn down.
fn gutter(sub: &Built) -> (Vec<(u16, String, Color)>, Vec<u16>) {
    sub.dispatch_tick(one());
    let mut times = Vec::new();
    let mut rule = Vec::new();
    for cmd in sub.frame_output() {
        match cmd {
            DrawCmd::Text { x: 0, y, text, fg, .. } if is_time(&text) => times.push((y, text, fg)),
            DrawCmd::Text { x: 9, y, text, .. } if text == "│" => rule.push(y),
            DrawCmd::Rect { .. } => panic!("the band paints no background: {cmd:?}"),
            _ => {}
        }
    }
    (times, rule)
}
```

  Check the field names of `DrawCmd::Text` (`fg` might be `Option<Color>` or be called something else) in `neotmux-proto/src/draw/`, and adjust the tuple type to match.

  Update the three tests:
  - `lines_that_came_together_share_one_time_written_once`: after `keys(&sub, &["T"])`, call `banded(&sub)`. Then expect one time on row 0 and the rule on rows `0..SIZE.rows`, because the rule is drawn the full height of the band.
  - `nothing_is_stamped_on_the_alternate_screen`: after `keys` and `banded`, `times` is empty.
  - `a_line_after_a_wait_is_coloured_by_it`: after `keys` and `banded`, the time on row 1 has `fg == Color::Rgb(0xf9, 0xe2, 0xaf)` and the one on row 0 does not.

  Add one test for the columns being given back:

```rust
/// Hidden, the plugin holds no band, so the panes get their columns back.
#[test]
fn hiding_the_times_gives_the_columns_back() {
    let (_dir, sub) = built(SHOWN);
    assert_eq!(sub.pane_band(), 0);
    keys(&sub, &["T"]);
    assert_eq!(sub.pane_band(), 10);
    keys(&sub, &["T"]);
    assert_eq!(sub.pane_band(), 0);
}
```

  (`Built` derefs to `LuaSubsystem`, so `sub.pane_band()` resolves.)

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p neotmux-lua --test stamps`
Expected: FAIL. `pane_band()` stays 0, and a `Rect` is drawn.

- [ ] **Step 3: Rewrite `runtime/lua/plugin/stamps.lua`'s header, `M`, `draw`, the tick and `toggle`.** `every_pane`, `numbers`, `record` and the `pane_closed` handler are unchanged.

Header (replace lines 1-14):

```lua
-- When every line of every pane arrived, and how long each one kept you waiting.
--
-- A terminal forgets the one thing about a line it cannot get back: when it came. This keeps
-- it, for every pane in every window, and `C-b t` (or the palette's `timestamps`) shows it in a
-- band down each pane's left edge — the time a line arrived, where it differs from the line
-- above, yellow or red when the pane sat still before it:
--
--   require('plugin.stamps').stall = { slow = 2, stuck = 10 }   -- seconds: yellow, then red
--
-- The band is the pane's own columns, not drawn over them: while it is shown, every pane's
-- shell is that many columns narrower, and hiding it gives them back. A time is known per tick,
-- not per line — lines that came together get the same one — and only a line that is new gets
-- one: a progress bar rewriting its row keeps the time it first appeared, and nothing is kept
-- while a pane is on its alternate screen. A line that falls off the pane's history takes its
-- time with it.
```

`M`:

```lua
local M = {
  shown = false,
  -- How long a pane sat before a line for the band to say so, in seconds.
  stall = { slow = 2, stuck = 10 },
  -- `HH:MM:SS`, a space, and the rule.
  width = 10,
  colors = {
    time = "#6c7086",
    rule = "#45475a",
    slow = "#f9e2af",
    stuck = "#f38ba8",
  },
}

local band
```

`draw`:

```lua
local function draw(ctx, pane)
  local it = panes[pane]
  local rect = ctx:pane_band(pane)
  if it == nil or rect == nil or it.top == nil then
    return
  end
  local rule = rect.x + rect.w - 1
  for row = 0, rect.h - 1 do
    ctx:text(nil, rule, rect.y + row, "│", { fg = M.colors.rule })
    local line = it.top + row
    local at = it.at[line]
    local before = it.at[line - 1]
    if at and before ~= at then
      local waited = before and at - before or 0
      local fg = M.colors.time
      if waited >= M.stall.stuck then
        fg = M.colors.stuck
      elseif waited >= M.stall.slow then
        fg = M.colors.slow
      end
      ctx:text(nil, rect.x, rect.y + row, os.date("%H:%M:%S", at):sub(1, rect.w - 2), { fg = fg })
    end
  end
end
```

`toggle`:

```lua
function M.toggle()
  M.shown = not M.shown
  if M.shown then
    band = g.reserve_pane("left", M.width)
  elseif band then
    band:remove()
    band = nil
  end
end
```

The tick is unchanged. It already draws only while `M.shown`, and `draw` now draws nothing for a pane with no band.

- [ ] **Step 4: Run to see it pass**

Run: `cargo test -p neotmux-lua --test stamps && stylua --check runtime && luacheck -q runtime`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add runtime/lua/plugin/stamps.lua neotmux-lua/tests/stamps.rs
git commit -m "C-b t shows the times in a band the panes give up for them, rather than over what the panes wrote"
```

---

### Task 6: Docs, changelog, the whole check, and the real screen

**Files:**
- Modify: `docs/plugins.md` (the Registering table near line 95, a new "### Reserving columns of every pane" after "### Reserving a band" around line 249, the `ctx` method list around lines 186-200, and the `stamps` row of the shipped defaults table)
- Modify: `CHANGELOG.md` (a new `### ` entry at the top of `## Unreleased`)

- [ ] **Step 1: Write the docs.**
  - Registering table: add the row ``| `neotmux.reserve_pane(edge, cells)` | nothing runs; it takes columns down the left of every pane (below) |``.
  - New section after "### Reserving a band". Use the spec's §2 code block. It must cover:
    - one band per source, and the 16-column cap across sources
    - only `'left'`, anything else raises
    - `ctx:pane_band(pane)` answers surface coordinates, or `nil`
    - draw with `ctx:text(nil, ...)`
    - a pane too narrow keeps its columns
    - `ctx:pane_rect` / `neotmux.pane.rect` are the content, and `ctx:pane_outer` is the rect with the bands
  - `ctx` method list: add `ctx:pane_band(pane)` and `ctx:pane_outer(pane)`.
  - Shipped defaults, `stamps` row: "`C-b t` shows when every line of every pane arrived, in a band down each pane's left edge that the pane gives up while it is shown".
  - `CHANGELOG.md`:

```markdown
### The times have a band of their own

`C-b t` no longer paints over the first nine columns of every pane. While the times are shown,
every pane's shell is ten columns narrower and the times sit in the columns it gave up, their
text yellow or red after a wait rather than the cell behind them. Hiding them gives the columns
back. `require('plugin.stamps').colors.gutter` is gone and `rule` is new.

The band is `neotmux.reserve_pane('left', cells)`, which any plugin can hold: `ctx:pane_band(pane)`
answers where a source's columns of a pane are, and `ctx:pane_outer(pane)` the pane with its
bands, which the borders and the prompt now draw around. `ctx:pane_rect` and
`neotmux.pane.rect` are what they were — the part of the pane the shell writes in.
```

- [ ] **Step 2: Run the whole check**

Run: `make check`
Expected: fmt, stylua, luacheck, clippy and `cargo test --workspace` all pass. Fix what they report and re-run until clean.

- [ ] **Step 3: Exercise it on a real screen.**
  - Build: `cargo build -p neotmux`.
  - In a scratch tmux-free terminal (or with `tools/` capture scripts if present), start `target/debug/neotmux` with the worktree's `runtime/` on the layer path. Check how `Layers::resolve` finds the shipped layer, e.g. from the README "install" section.
  - Split with `C-b v`, then with `C-b -` in the right pane.
  - Run `for i in 1 2 3; do echo $i; sleep 3; done` in one pane, and `ls` in the others.
  - Press `C-b t`. Check four things:
    - every pane's text moves 10 columns right, and `tput cols` in a pane reports its width minus 10
    - the times line up with each pane's rows, and the lines after a 3-second wait are yellow
    - the `│` rule runs down every pane
    - the horizontal border under the top-right pane is unbroken from its left edge
  - Press `C-b t` again: every pane gets its columns back.
  - Capture the screen before and after for the report.

- [ ] **Step 4: Commit**

```bash
git add docs/plugins.md CHANGELOG.md
git commit -m "docs for neotmux.reserve_pane and the times in their own band"
```

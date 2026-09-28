# neotmux — a band down every pane's edge

Date: 2026-09-28
Status: proposed
Scope: a host primitive, `g.reserve_pane(edge, cells)`, that takes columns off the left of
every pane, and `plugin.stamps` rewritten to draw its times there instead of over the pane.

## 1. Goal and non-goals

### Goal

`C-b t` paints nine columns of every pane with times and yellow and red blocks, over whatever
the pane has there. The first columns of every line are unreadable while it is shown, and the
blocks are the loudest thing on the screen.

After this: while the times are shown, each pane's shell is that many columns narrower, the
times sit in the columns it gave up, and nothing a pane wrote is covered.

`neotmux.reserve` cannot do this. A band belongs to the surface: a left band beside a
horizontal split lines up with the left pane's rows and not the right one's.

### Non-goals

- How a line's arrival is recorded. `record` in `plugin.stamps` keeps guessing from the
  cursor, per tick, in seconds; that it is wrong for a program that draws in place without the
  alternate screen is a separate problem.
- A band on one pane only. Every shipped user wants every pane; a band per pane is a later
  argument if a plugin needs it.
- Edges other than the left. `edge` is in the signature so adding one does not break a caller;
  anything but `'left'` is refused for now.

## 2. The API

```lua
local g = neotmux.group('plugin.stamps')
local handle, message = g.reserve_pane('left', 10)   -- also neotmux.reserve_pane
handle:remove()
handle:resize_to(12)

g.on_tick(function(ctx)
  for _, pane in ipairs(neotmux.pane.list()) do
    local band = ctx:pane_band(pane)      -- { x, y, w, h } in surface coordinates, or nil
    local outer = ctx:pane_outer(pane)    -- the pane with every band, or nil
  end
end)
```

It is `reserve` again, one level down, and follows it wherever it can:

- One per source; a second is refused until the first is removed.
- Granted in load order, filled from the pane's edge inwards, and `resize_to` keeps a band's
  place in that order.
- Takes effect on the next frame. The handle persists; the drawing does not.
- Refused with `nil, message` past a cap on the edge: 16 columns, summed over every source.
  A pane band is a gutter of text, and past that it is the pane.
- Held by the group, so `config-reload` takes it back like every other registration.

`ctx:pane_band(pane)` answers the caller's own band in that pane. It is `nil` when the caller
holds none, when the pane is not shown, or when the pane is too narrow to have its bands this
frame (§3). A `nil` means draw nothing in that pane this tick.

Drawing into it is `ctx:text(nil, ...)` and `ctx:rect(nil, ...)` at the band's coordinates,
the same as a surface band. A pane-relative draw stays relative to the pane's content, so
`ctx:text(pane, 0, y)` still lands on the pane's first column of output.

## 3. What a pane's rect means

Today a pane has one rect, and the pty's size, where its grid is drawn, `ctx:pane_rect`,
`neotmux.pane.rect`, and whether it can be split are all read from it.

After this a pane has two:

- **outer**: what `assign()` cuts, unchanged. The layout tree, split and resize work in it.
- **content**: outer less the pane bands. This is what the pty is sized to and where the grid
  is drawn.

Every existing reader gets **content**: `ctx:pane_rect`, `neotmux.pane.rect`, `PaneFrame.rect`,
`Frame.rects`, `Geometry.panes`, and the spawn size of a new pane (`pane_size()`). Everything
that reads grid coordinates — copy mode, hints, the cursor, pane-relative `ctx:text` — then
keeps working without a change.

Outer is new, and only for what draws around a pane rather than in it: `ctx:pane_outer(pane)`.
`plugin.tmux.borders` moves to it, since its horizontal rules would otherwise stop short of
the band, and so does `neotmux.prompt`'s bounding box.

**Too narrow.** A pane whose outer width is less than the minimum pane width plus the bands
gets none of them this frame, and its content is its outer rect. The decision is per pane:
the others keep theirs. This is `applied()`'s all-or-nothing, per pane rather than per
surface, because one small pane should not take the times off every other.

**Split and resize** are refused against the minimum plus the bands, so a split cannot leave a
new pane that is only a band.

**Zoom** gives the zoomed pane the pane area as outer, and derives content the same way.

The derivation is one function in `PaneLayout`, next to `applied()`, and every rect above goes
through it.

## 4. From Lua to the layout

The field chain `reserved` and `gutter` already travel, with one more field:

- `Tock` and `Setup` carry `pane_band` (columns on the left, summed over every source).
- `Surface` carries it into `Windows` and `PaneLayout`.
- `Session::apply` compares it with the last one it saw, the way `reserved_changed` does, and a
  change runs `apply_geometry`: resurface, lay out, fit, `resized`.
- `Geometry` carries `outer`, every shown pane's outer rect, beside `panes`. A pane whose outer
  rect is wider than its content got its bands this frame; the Lua side cuts each source's band
  out of the outer rect, filled from the edge inwards in grant order.

## 5. `plugin.stamps`

What it records does not change. What it draws does:

- `C-b t` reserves the band; pressing it again removes it, so the panes get their columns
  back. Hidden, it holds nothing.
- The band is ten columns: `HH:MM:SS`, a space, and a `│` in the dim colour down the last one,
  on the pane's own background rather than a panel colour.
- A time is shown where it differs from the line above, as now.
- A line that kept the pane waiting colours its time's text, yellow past `stall.slow` and red
  past `stall.stuck`. No cell's background is filled.
- A pane too narrow for the band this frame shows no times.

## 6. Testing

- `neotmux-layout`: outer and content for a split, a zoomed pane, a pane too narrow for the
  band, and split/resize refused against minimum plus band.
- `neotmux-lua`: `reserve_pane` refusals (second per source, over the cap, an edge other than
  left), `resize_to`, removal by `config-reload`, and `ctx:pane_band` / `ctx:pane_outer`
  answers from a host's `Step::Bands`.
- `neotmux-server`: a change in `pane_reserved` resizes the ptys, and a pane spawned while a
  band is held starts at the content width.
- By hand: `C-b t` in a session with a horizontal and a vertical split, a shell in each; the
  shells narrow, the times line up with each pane's rows, the borders are unbroken, and `C-b t`
  again gives the columns back.

## 7. Docs

`docs/plugins.md`: `reserve_pane`, `ctx:pane_band`, `ctx:pane_outer`, and that `pane_rect`
means the content. The shipped defaults table's `stamps` row. `CHANGELOG.md`.

# neotmux — floating windows

Date: 2026-09-21
Status: proposed
Scope: GitHub issue #1 — "Floating windows: a bound that is a free rectangle". Today a
plugin draws in a band it reserved on an edge, or over a pane, and coordinates given with a
pane are pane-local and clipped to that pane's rect. There is no third option, so a
rectangle that floats over the layout — spanning panes, owing its position to nothing —
cannot be expressed. This design adds exactly that: one primitive, on the client, with no
wire changes.

## 1. Goal and non-goals

### Goal

A plugin can grant itself a free rectangle in surface coordinates,
`neotmux.float({ x =, y =, w =, h = })`, get back a handle with `move_to` / `resize_to` /
`remove` (the shape `reserve` already carries), and draw into that rectangle with its own
local coordinates, the same immediate-mode bargain it uses for a band or a pane. The
rectangle overlays the layout. It is deliberately not a band: a band takes cells away from
the panes, and a float that did that would be a split.

### Non-goals

- **A real window.** No PTY behind it and no separate input target. A float is not
  *addressable* — its owning plugin holds the keyboard for it with the existing `on_key`
  mechanism, which is how copy mode is modal today. nvim's "float is a focusable window" is
  the opposite model; neotmux keeps one modal mechanism.
- **Anchored or relative placement** (nvim's `relative: pane/editor` and friends). Absolute
  surface cells only. A plugin that wants a centred modal re-positions on `resized` with
  `move_to`; the host does not re-anchor for it.
- **An explicit z-index.** One host-owned order (grant order) decides both drawing depth and
  keyboard precedence. A numeric `zindex` is a later addition if two floats ever need an
  order the grant order cannot say.
- **Server visibility, wire traffic, or a protocol bump.** A float is client-side. The
  server keeps panes, shells, layout and the focused pane; it does not learn that a float
  exists.
- **Mouse and multiple sessions.** Still out; a float neither depends on them nor enables
  them.

## 2. What changes, in one paragraph

The client's Lua host (the same layer that already owns `Reservations`) gains a second,
small registry: an ordered set of floats, each a rect and an owner. Drawing gains one stage
between what it does now — panes, then the bands and other surface draws, then notices — and
nothing else: floats, in grant order, last one on top, each clipped to its rect. Key
dispatch gains one rule: `on_key` callbacks whose owner holds a float are offered before
the others, in that same float order. That is the whole of it: no message the wire has
never seen is seen, and `PROTOCOL_VERSION` does not move.

## 3. The API

```lua
local win, message = neotmux.float({ x = 10, y = 5, w = 40, h = 12 })
win:move_to(20, 6)        -- the same rect, elsewhere on the surface
win:resize_to(50, 20)     -- the same rect, a different size
win:remove()              -- idempotent, like every other handle
```

- Two-value errors, the `reserve` convention: a bad grant is `nil` plus a message and
  nothing is recorded. `x`/`y` are integers `>= 0`; `w`/`h` are integers `>= 1`. A rect
  that stretches past the surface is granted and clipped, not refused: the surface is
  allowed to be smaller than the request, and the float's own bounds are what a drawing
  into it is clipped to.
- It takes effect on the **next** frame, like `reserve` and `pane.split`. It may be made
  during `init.lua`, for the same reason a band may: the grant is a pending rectangle
  until the first frame there is a surface to sit on.
- **The bargain is the same as `reserve`: the handle persists and the drawing does not.**
  Every tick, for as long as the float exists, the plugin draws its whole content into it:
  immediate mode. Draw nothing this tick and the float is an empty rectangle this frame.
- Drawing names the float as the first argument, exactly as a pane is named today:
  `ctx:text(win, x, y, s, opts)`, and the same for `rect`, `tint` and `line`.
  Coordinates are **float-local**, `(0, 0)` at the float's own top-left, and a command that
  runs past the rect is clipped, the way a pane-local command is clipped to its pane.
  `ctx:float_rect(win)` answers the owner's own float as `{ x, y, w, h }` in surface
  coordinates — or `nil`, in which case the draws for it this tick are dropped and all of
  it means one thing, draw nothing, which is the bargain `ctx:reserved()` already makes
  for bands.

## 4. Coordinates, clipping, order

- Coordinates are the **surface's**, the same frame `ctx:size()` reports without any
  reservation, `(0, 0)` at the top-left, bands included. A float and a band may occupy the
  same cells: the float is an overlay, and what is drawn later is what is seen.
- **Floats do not take cells away from the panes.** The layout, `ctx:size()` and everything
  computed from them are untouched, which is the whole point of the difference from
  `reserve`: a band is a claim on the surface, a float is a claim on nothing.
- Clipping is at the float's own rect on the way in and at the surface on the way out;
  there is no third edge in between. A shrink of the terminal clips a float whose corner
  has left the surface, and nothing else moves.
- **A resize moves nothing.** A float's rect is absolute cells and stays where it was
  granted; only the clipping changes. Re-anchoring, re-centering and everything else that
  wants to follow the surface is the owner's work on the one event that tells it the
  surface has changed, and it already has that event (`resized`, the same one a band uses
  to learn its cells were taken away).
- **Order.** The host owns one order, grant order, and it is the whole of the question:
  a later float is drawn over an earlier one, for as long as both exist. There is no
  reordering today; the way to bring one back is the way every other reservation comes
  back, `remove` and a new grant, which makes it the one on top. The drawn layers, bottom
  to top, are panes, bands, floats in grant order, notices.

## 5. The keyboard

The keyboard is the client's, and this section says where a float sits in the one order the
client already keeps:

1. `on_key` callbacks whose owner holds a float, **in float order, the topmost float
   first**. A plugin that holds several floats sits once, at the position of its topmost
   one.
2. the remaining `on_key` callbacks, in registration order as now,
3. then the keymap binding,
4. then the key crosses the socket to the server, to whichever pane has the focus — which
   stays the server's, because it is a pointer into panes the server owns, and the server
   keeps it valid when the panes it points at go away.

Nothing else changes. A float does not hold the keyboard any more directly than a band
does: the owner's callback returns `true` and the key is spent, or it does not and the key
goes on. That is the one modal mechanism this design has, the same one copy mode uses, and
the reason there is no "focused float" to keep valid. And the moment a plugin wants two
of its own floats to disagree about a key, the answer is the drawing's: the one on top is
asked first.

## 6. What a detach does not carry

A float is plugin state, and it dies the way plugin state dies:

- **On a detach**, a client that leaves with `C-b d` or by accident takes its floats with
  it. Re-attach re-runs the `init.lua` chain, and a float comes back only if the plugins
  that hold one ask for it again — the same bargain a raised mode or a copy register keeps.
- **When its owner goes.** A unload, or the unsubscription an over-budget plugin earns,
  takes the owner's floats with it, the same fate an owner's bands keep.
- **Not on a resize**, which is the server telling the client its surface changed rather
  than the client going away. The rect stays where it was; only its clipping changes (§4).

## 7. Error handling

| failure | handling |
|---|---|
| `neotmux.float` on a non-integer, a negative, or a zero-sized rect | `nil` plus the message, nothing recorded — the `reserve` table |
| a float drawn into after its `remove` this frame | dropped, `ctx:float_rect` answering `nil` — the band bargain |
| a float the owner's unsubscription takes mid-frame | dropped from the draw and from the offer on the frame it is gone |
| the owner's `on_key` consumed, then raised | reported with the owner's source, as every other callback's error is |
| two floats over the same cells | the later one on top; both remain, both can consume, in order |
| a float larger than the surface | granted and clipped; the request was not mistaken, the surface was smaller |
| the owner's client, with its floats, dies | the floats die with it, and nothing that survives — panes, shells, the session — changes |

## 8. Tests

- A granted float draws over the pane and the band beneath it; a second draw goes over the
  first; a `remove` puts both back, checked through the drawn surface as the band tests
  check theirs.
- A float-local draw is clipped to the rect; a rect past the surface is clipped to the
  surface, and a `resize_to` that shrinks it clips what it drew before.
- **The keyboard canary**: with no floats, the offer order is byte-for-byte the order it
  is today. With two floats and both owners consuming, the topmost is asked; the topmost
  declining hands the key to the lower one; both declining falls to the registration
  order, then the binding, then the socket. The canary exists because the whole point of
  §5 is that a float is *inserted into* an order that worked before, and an order that
  changed on its own is the one failure no drawing test will catch.
- A detach with a float open, and a re-attach that shows the panes and not the float until
  the plugin's `init` asks for it again.
- The server never hears it: the same session, the same frames, the same handshake number,
  with a float open on one client and the session re-attached on another.

## 9. What this does not touch

- The wire, `neotmux-proto`, `PROTOCOL_VERSION`, and the server: no float crosses a
  socket, and the one that would cost a handshake has been deliberately kept off it.
- `reserve` and the layout it claims: a band still takes cells, still bounds at four rows
  and forty columns, still fills an edge outside-in. A float is the claim-on-nothing next
  door.
- The focused pane, which stays the server's because it is a pointer into server-owned
  panes, and moves the way it moves today when a pane goes.
- Notices, which stay on top of everything, the way a message that must be read should be.
- The per-frame budget, which stays the one meter: a float adds no new kind of
  instruction, so a float that draws ten thousand cells is the overrun it already is.
- Multiple sessions, named ones, listing them, and two clients on one — all still out in
  the same place they are now, and none of them is what makes this design right or wrong.

## 10. Timing

This is new ABI surface, and §1 of the design spec cuts v0.1 at the line where a plugin
author can write against the ABI. A float is deliberately that kind of surface: the
moment an outside plugin is drawn into one, its coordinates, its clipping, its order in
the draw and its position in the keyboard cannot be moved without breaking the drawing
that assumed them. So it lands before v0.1, not half of it in v0.1 and the rest behind a
`PROTOCOL_VERSION` nobody was keeping.

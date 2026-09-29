# neotmux — buffers: what a plugin shows is held by the host, and changed only when it changes

Date: 2026-09-29
Status: proposed
Scope: text buffers, highlight groups, a window (band or float) showing a buffer, the events a
plugin needs to keep one up to date, and `plugin.herdr.sidebar` moved onto all four. Shaped
after Neovim: a buffer holds lines, extmarks colour them by highlight group, a window shows a
buffer, and autocmds say when to change it.

## 1. Goal and non-goals

### Goal

The sidebar is the most expensive thing in the Lua layer, and it does all of that work on every
tick whether or not anything it shows has changed. Measured on 2026-09-29 with the palette's
`costs`, release build, 30 s idle after setup:

| sessions, panes | sidebar mean | sidebar longest | next source's mean |
|---|---|---|---|
| 1, 4 | 1.02 ms | 4.37 ms | 0.24 ms |
| 6, 24 (2 claude) | 1.96 ms | 7.77 ms | 0.20 ms |
| 12, 48 (2 claude) | 2.37 ms | 7.97 ms | 0.17 ms |

The longest tick at 48 panes is at the 8 ms tick budget. Profiled with `os.clock` around each
phase, a 1-session tick is spread across drawing (0.49 ms: rows 0.27, the rule 0.11), the tree
(0.25 ms), the walk of every session (0.11 ms) and the agents (0.08 ms); at 48 panes the two
walks, `spaces.entries` (0.78 ms) and `agents.collect` (0.55 ms), are over half of it. Every
tick pays all of it, including the ones a key press or a pane's output caused and that changed
nothing the sidebar shows.

herdr (checked at `9dc3a1d`) rebuilds its whole sidebar model when it does rebuild, but a frame
caused only by pane output takes a retained-surface path that never touches the sidebar, and a
rebuilt snapshot equal to the last one is not sent.

After this:

- A plugin writes what it shows into a **buffer** the host keeps, and the host draws it every
  frame into the **window** showing it. No Lua runs to redraw an unchanged buffer.
- A plugin changes its buffer from **events**: a pane changed, a session or window came, went or
  was renamed, focus moved, the surface resized, a timer came due.
- Colours are **highlight groups** the host holds, published by `neotmux.style`. A theme change
  recolours every buffer without its plugin doing anything.
- The sidebar's cost on a tick where nothing it shows changed is zero: it has no `on_tick` work
  for its rows at all.

### Non-goals

- Panes as buffers. A pane is a terminal buffer in Neovim's terms, and the layout's slots are
  windows; that would let a float show a pane or a slot show a text buffer. The layout tree holds
  `PaneId`s bound to PTYs, and splitting, resizing, focus and the protocol all assume it, so it is
  its own project. This design leaves room for it (§2.1: a buffer has a kind; §2.3: what shows one
  is a window) and does not start it.
- Moving anything but the sidebar onto buffers. The status bar, `neotmux.list`, the palette and
  the prompt stay immediate-mode.
- Scrolling in the host (`topline`), folding, virtual text, signs, conceal, and editing a buffer
  from the keyboard. The sidebar writes the rows it wants seen.
- Extmarks that move with edits. §2.2 says what an extmark does when its lines are replaced.
- A `state_changed` event from `neotmux.state`. The sidebar judges the panes a `pane_changed`
  named; chime and watch keep what they do.
- Making the one remaining walk cheaper for the other plugins that still do it.

## 2. Design

### 2.1 Buffers

```lua
local buf = neotmux.buf.create()                       -- also g.buf.create(): the group owns it
neotmux.buf.set_lines(buf, 0, -1, { ' ● work', '   main ↑2' })
neotmux.buf.line_count(buf)                            -- 2
neotmux.buf.delete(buf)                                -- idempotent
```

- A buffer is a list of lines, each a string. Indexing is Neovim's: zero-based, `end` exclusive,
  `-1` the end, so `set_lines(buf, 0, -1, lines)` replaces everything and `set_lines(buf, 2, 2,
  lines)` inserts before line 2.
- `set_lines` raises on an index past the end (Neovim's `strict = true`), a non-string line, or a
  line holding `\n`. These are typos, the same class `reserve` raises for.
- A buffer has a kind, `text`, and only `text` exists. The kind is there so a terminal buffer can
  be added without renaming anything (§1, non-goals).
- The owner is the source that created it, or the group it was created through. A failed load,
  `config-reload` emptying the group, or the unsubscription an over-budget plugin earns deletes
  it, the same fate a band or a float has. A deleted buffer's handle raises on every call but
  `delete`.
- One VM serves every session, so a buffer is not a session's. The sidebar has one buffer, shown
  in every session's band.

### 2.2 Extmarks and highlight groups

```lua
local ns = neotmux.buf.namespace('plugin.herdr.sidebar')
neotmux.buf.set_extmark(buf, ns, 0, 1, { end_col = 4, hl_group = 'sidebar:working' })
neotmux.buf.set_extmark(buf, ns, 0, 0, { line_hl_group = 'sidebar:here' })
neotmux.buf.set_extmark(buf, ns, 1, 3, { end_col = 7, hl_group = 'sidebar:branch', bold = true })

neotmux.hl.set('sidebar:branch', { fg = '#cba6f7' })     -- what neotmux.style does (below)
```

- `row` and `col` are zero-based; `col` and `end_col` count **columns**, as `neotmux.width` does,
  not bytes. A column past the line's end is clamped to it.
- `hl_group` colours the range with that group's `fg`, `bg` and `bold`. `line_hl_group` paints
  the group's `bg` across the whole width of the window on that row, under the text — the `fill`
  the sidebar's rows have today. `bold = true` on the extmark is an override for that range.
- Later extmarks win over earlier ones where they overlap, per channel: an extmark naming only an
  `fg` keeps the `bg` under it, the rule `ctx:tint` has.
- **Replacing lines drops the extmarks on them.** An extmark on a line `set_lines` replaced or
  deleted is gone; one below the range moves with its line. This is the one departure from
  Neovim, where an extmark collapses to the start of the range: a plugin that rewrites a row
  always sets that row's colours again.
- `namespace(name)` answers the same id for the same name. `clear_namespace(buf, ns, start,
  end)` removes that namespace's extmarks on those lines.
- A highlight group is a flat name. `neotmux.hl.set(name, { fg, bg, bold })` defines or replaces
  it; colours are `#rrggbb` or a palette index, as everywhere else. A group nobody set draws in
  the terminal's defaults.

**`neotmux.style` publishes the groups.** For every selector it has a default or a set for, and
every part of it, it calls `neotmux.hl.set(selector .. ':' .. part, resolved)` — so
`style.default('sidebar', { branch = 'mauve' })` defines `sidebar:branch`. It publishes again
whenever a `default` or a `set` changes and whenever the theme's generation moves, which
`neotmux.theme` tells it through an internal hook rather than `style` polling. `style.get` keeps
working for immediate-mode plugins.

### 2.3 Windows

```lua
local band = neotmux.reserve('left', 26)
band:set_buf(buf)            -- the band shows the buffer from the next frame
band:set_buf(nil)            -- and stops
local win = neotmux.float({ x = 10, y = 5, w = 40, h = 12 })
win:set_buf(buf)
```

- A band's handle and a float's handle are windows. Each shows at most one buffer; one buffer may
  be shown in any number of windows. `set_buf` answers `true`, or raises on a deleted buffer.
- Line 0 is the window's top row. Rows past the buffer's end are left as the window's background;
  lines past the window's height are not drawn. A line wider than the window is cut at its
  width, and a wide character that would straddle the edge is left out. Ellipses are the
  plugin's: it knows the width when it writes.
- **Stacking.** Within a window's cells, the buffer is drawn first and the owner's immediate-mode
  drawing (`ctx:*`) for that frame goes over it. Windows stack as bands and floats already do.
- A window the terminal is too small for this frame (`ctx:reserved()` is nil) shows nothing, the
  same as today.

### 2.4 Drawing in the host

Frame assembly, beside `Buffers::applicable` in `neotmux-lua/src/draw/`, turns each shown
buffer into `DrawCmd`s against the window's rectangle **of this frame**: a `Rect` for each
`line_hl_group` row, then `Text` runs split at extmark boundaries. No Lua runs.

- Immediate-mode drawing is replayed only while the epoch it was committed under matches, and is
  lost on a geometry change. A buffer is projected afresh every frame, so it has no such problem.
- The frame is recomposed when a buffer shown in the session changed, the same way a changed
  overlay makes the session dirty today.
- A buffer changed in session A's event is drawn in session B when B next makes a frame, which
  for an attached, idle B is within the 250 ms idle tick. That is the lag the sidebar has today.
- Widths are `neotmux_proto::screen::width`, the measure `neotmux.width` and the renderer use.
- New types follow the one-file-per-type rule: `TextBuffer`, `TextBuffers` (the store), `Extmark`,
  `HlGroups`, each in its own file under `draw/`; the Lua bindings are `api/buf.rs` and
  `api/hl.rs`.

### 2.5 Events

What the sidebar needs to know, as events, in the existing `on_event` table:

| name | fields | when |
|---|---|---|
| `pane_changed` | `pane` | that pane's terminal was fed since the last frame. **Once per pane per frame**, with no bytes |
| `session_created` | `session` | a session started |
| `session_closed` | `session` | a session ended |
| `session_renamed` | `session`, `from` | a session was renamed |
| `window_created` | `window` | a window was made |
| `window_closed` | `window` | a window's last pane went |
| `window_renamed` | `window` | a window was named or unnamed |

- `pane_changed` is Neovim's `TextChanged` beside its `on_bytes`: `pane_output` still fires per
  chunk with the bytes, for a plugin that reads them, and `pane_changed` is coalesced by the host
  at the end of the drain. A plugin that only wants to know is not handed every chunk.
- `pane_opened`, `pane_closed`, `focus_changed` and `resized` exist and are used as they are.
- **Across sessions.** Events are dispatched bound to the session they happened in, as today, so
  `neotmux.session.current()` and `pane.*` inside a handler speak of that session. Session events
  are dispatched bound to the session they name; `session_closed` is dispatched unbound, in the
  next dispatch of any session, with only the name; inside it `neotmux.session.current()` is nil.
- `window_renamed` and the session events are raised where the server applies the op that
  caused them, not by the Lua call asking for it: a rename refused or queued is not an event.

### 2.6 The sidebar on buffers

`plugin.herdr.sidebar` keeps one model: per session, its windows, panes, the state of each pane,
its agent, its branch and counts; and the agents across all sessions. It changes the model from
events, and rewrites the buffer when the model changed:

- `pane_changed` / `pane_opened` / `pane_closed`: judge that pane again (`state.of`, which reads
  the screen only when the revision moved), update its session's rollup and the agents list.
- `focus_changed`, the window and session events: rebuild that session's entry and tree.
- `resized`, the band's width changing, fold, the navigate cursor: re-lay out the rows; the model
  is unchanged.
- **Time.** A pane whose state can change with no output — `working` inside `working_for`, a
  detector holding a settle — is re-judged by a `neotmux.timer.every(1000)` over only those
  panes; the timer judges nothing when there are none. `done` changes when the pane is looked at,
  which is `focus_changed`.
- **Git** is asked by the same timer at `M.refresh`, a job per directory as now; a result that
  differs from the model's rewrites the rows.
- Rewriting is the code that exists: `row`, `fit`, the ellipsis and the tree build rows as they do
  now, and the header, the separators, the rule down the right edge and the fold corner are written
  into the buffer as lines and extmarks instead of drawn. One `set_lines(buf, 0, -1, …)` and its
  extmarks per change.
- `on_tick` keeps only what is drawn over the buffer while it is up: the navigate bar.
- Tokens change shape: a token answers text and a highlight group name (`'sidebar:branch'`)
  instead of a colour. This breaks a user's own token in `init.lua`; `docs/plugins.md` and the
  CHANGELOG say so and show the new shape.

`plugin.herdr.sidebar.sessions`, the old tree section, moves the same way, since `spaces` draws
its tree with it.

## 3. Errors and lifetime

- A raising event handler is reported once and the buffer keeps what it last had: the host shows
  the last good content, not an empty band. This is the reverse of immediate mode, where an
  overrun source's drawing is discarded, and it is deliberate — a notification about the Lua layer
  failing already has its own surface, and a sidebar that froze is easier to diagnose than one that
  vanished.
- An over-budget plugin is unsubscribed as today, and its buffers and windows go with it.
- `config-reload` empties the group; the sidebar comes back by building its model from nothing,
  the same path it takes at load.

## 4. Testing

- Rust integration tests in `neotmux-lua/tests/`: `buf.rs` (indexing, strict errors, extmark
  ranges in columns, replacing lines drops their extmarks, ownership and delete), `hl.rs`
  (groups, unknown group, a theme change reaching a shown buffer with no plugin code running),
  and window tests in `reserve.rs` and the float tests (projection, clipping by columns, a wide
  character at the edge, stacking under immediate drawing, a geometry change keeping the buffer).
- Event tests: `pane_changed` fires once per pane per frame however many chunks arrived, and the
  session and window events fire from the server's ops (`neotmux-server` tests where the op is
  applied).
- `neotmux-lua/tests/sidebar.rs` keeps asserting on `DrawCmd`s, which the host now emits from the
  buffer; the existing tests pass unchanged except where they build tokens. New: a tick with no
  event does not touch the buffer, and a pane's output that changes its state rewrites that row.
- On the real screen: the three configurations of §1, measured again with `costs`. Success is the
  sidebar's mean under 0.05 ms at 48 panes on idle ticks, and no worse than today on a tick where
  a state changes.

## 5. Order

One plan, in this order, each step shippable on its own: highlight groups and `neotmux.style`
publishing them; buffers, extmarks and windows with the host drawing them; the events; the
sidebar moved onto them.

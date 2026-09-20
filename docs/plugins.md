# Writing a neotmux plugin

neotmux has no plugin system. It has an API, and a convention for where Lua files live. A
plugin is a file that calls the API; it is a plugin only because you call it one. Nothing
registers it, nothing declares it, and the program never learns the word.

That is not a slogan — it is the reason things work the way they do below. There is no manifest
because there is nothing to declare to. There is no enable/disable because loading a file is
what "enabled" means. There is no plugin name because the thing neotmux knows about is the
*chunk your code was written in*, which it names after the file's path.

## Where files go

```
~/.config/neotmux/
  init.lua                 -- run at startup
  lua/
    plugin/clock.lua       -- require('plugin.clock')
    plugin/mything/init.lua -- require('plugin.mything')
    mylib/helper.lua       -- require('mylib.helper')
```

`init.lua` runs at startup. Everything under `lua/` is on `package.path`, so `require` finds
it. `plugin/` is not special to the program — it is just where people put things, the way
nvim's `runtimepath` has one. `require('mylib.helper')` works exactly as well.

`$NEOTMUX_RUNTIME` can name extra directories, colon-separated, searched before your config.
The first directory that has a file wins, so a layer earlier in the list shadows a later one.

## Hello world

`~/.config/neotmux/lua/plugin/hello.lua`:

```lua
local band = neotmux.reserve('bottom', 1)

neotmux.on_tick(function(ctx)
  local row = ctx:reserved()
  if row then
    ctx:text(nil, row.x, row.y, 'hello', { fg = '#00ff88' })
  end
end)
```

`~/.config/neotmux/init.lua`:

```lua
require('plugin.hello')
```

Start neotmux and `hello` is along the bottom of the screen, on a row the panes no longer
have. Delete the `require` line and it is not. That is the whole lifecycle.

## The API

Everything hangs off the global `neotmux`. The table is sealed — you cannot add to it or
replace its fields.

### Registering

Each of these returns a handle with one method, `handle:remove()`, which is idempotent.

| call | when your function runs |
|---|---|
| `neotmux.on_tick(fn)` | once per frame, with a `ctx` (below) |
| `neotmux.on_event(name, fn)` | when that event fires, with a table of fields |
| `neotmux.on_key(fn)` | for every key, before any binding; return `true` to consume it |
| `neotmux.keymap.set(spec, fn)` | for that one key; return `true` to consume it |
| `neotmux.reserve(edge, cells)` | nothing runs; it takes a band of the surface (below) |

Every event that fires, and the fields it carries:

| name | fields | when |
|---|---|---|
| `resized` | `rows`, `cols` | the area available to panes changed — including when a band is reserved or released |
| `pane_output` | `pane`, `bytes` | that pane's child wrote something |
| `pane_opened` | `pane` | a split created it |
| `pane_closed` | `pane` | its child exited and the pane is gone |
| `focus_changed` | `pane` | focus moved to that pane |

There are no others. Subscribe to a name nothing sends and nothing happens, silently.

The three pane events fire against the tree they describe: a `pane_opened` handler calling
`neotmux.pane.list()` sees the new pane, and a `pane_closed` handler does not see the dead
one.

`keymap.set` returns `nil, message` when another source already holds that key. It *raises*
when the spec itself is unparseable: losing a race is something to handle, a typo is a bug.

Key specs are `C-`, `M-`, `S-` prefixes in any order plus a key: `C-b`, `M-Enter`, `F5`, `x`.
`x` and `X` are different keys, because that is what the terminal reports. `S-` does not
combine with a single character for the same reason.

### Drawing

Drawing happens only from a tick, through the `ctx` your callback is handed. It is
immediate-mode: **what you draw this tick is your entire output for this frame.** Draw nothing
and you show nothing. There is no clear call because you never need one.

```lua
neotmux.on_tick(function(ctx)
  local size = ctx:size()                      -- { rows = , cols = }: what the panes have
  ctx:text(nil, x, y, 'text', { fg = 2 })      -- nil pane = the whole surface
  ctx:rect(nil, x, y, w, h, { bg = '#202020' })
  ctx:tint(nil, x, y, w, h, { bg = 4 })        -- recolours, writes no glyph
  ctx:line(nil, x1, y1, x2, y2, { fg = 4 }, '|') -- no bg: it paints single cells
                                               -- the glyph is optional and defaults to '#'
  local r = ctx:pane_rect(pane_id)             -- { x = , y = , w = , h = } or nil
  local band = ctx:reserved()                  -- your own band, or nil (below)
end)
```

Colours are a palette index `0..255` or a `"#rrggbb"` string. Omit the table and you get the
terminal's defaults.

`ctx:line`'s glyph is one character: it fills one cell per step, so anything longer raises
rather than being truncated.

`ctx:size()` is the area available to the shells, so it shrinks by whatever is reserved at the
edges. Read it every tick rather than caching it.

**Coordinates are the surface's.** A `nil` pane means the whole terminal, reserved bands
included, and `ctx:pane_rect` answers in those same coordinates — a band at the top moves the
panes down, so a pane's rect is never relative to anything but the terminal itself.

`pane_gutter` is the same bargain as a reserved band, in the other direction: the host takes
the cells away from the panes and has no idea what goes in them. With a gutter of `g`, a
pane's right neighbour starts at `r.x + r.w + g` and the `g` columns from `r.x + r.w` belong
to nobody — which is where `plugin.borders` puts its dividers. At `0`, the default, the panes
tile the surface exactly.

**`ctx:rect` with no colours clears the area.** It is not a no-op — default background is a
colour like any other, and painting it is how you erase.

**`ctx:tint` is `ctx:rect` that does not erase.** Same rectangle, same coordinates, and that
is where the resemblance stops: `rect` writes a space into every cell it covers, and `tint`
writes nothing at all. It changes the colours of the cells that are there and leaves the
characters alone, which is what highlighting a region of a pane's text is.

```lua
ctx:tint(id, 0, row, r.w, 1, { bg = 4 })            -- the line stays readable
ctx:rect(id, 0, row, r.w, 1, { bg = 4 })            -- the line is now a blue bar
```

**A colour you leave out is a colour the cell keeps.** `{ bg = 4 }` recolours the background
and leaves whatever coloured the text — so a tint over syntax-highlighted output keeps the
highlighting. Name both and both change. There is no way to spell "put this channel back to
the terminal's default": omitting it means *keep*, not *reset*.

**A tint that names no colour raises.** `rect` with no colours is a deliberate erase, but a
tint with no colours could only be a mistake, so it is not quietly accepted.

**A cell nothing has written tints like any other.** The pane grid is full of cells the child
never touched; each one is a blank with the terminal's own colours, and tinting one recolours
that blank. No glyph appears where there was none — you get a coloured space, which is what
selecting past the end of a line looks like.

### Reserving a band

```lua
local handle, message = neotmux.reserve('bottom', 1)   -- 'top', 'bottom', 'left' or 'right'
handle:resize_to(2)                                    -- the same band, a different size
handle:remove()                                        -- give the cells back
```

The panes get what is left. A band is outside every pane's rect, so nothing a shell writes can
land in it and nothing in it is overwritten by a pane. That is the whole of what the status
row used to be, and it is why there is no `ctx:status`: a status line is a plugin that wants
one row at the bottom.

**The unit is the edge's own.** `top` and `bottom` are counted in rows, `left` and `right` in
columns. Nothing else about the two axes differs, which is why there is one call and not two.

A horizontal band spans the full width; a vertical one spans only what the horizontal bands
left. No two bands ever share a cell, and the corner goes to the status line, because that is
what a line across the terminal means.

`ctx:reserved()` answers **your own** band, as `{ x, y, w, h }` in surface coordinates, or
`nil`. It is per source: two plugins reserving the bottom get a row each, and the one that
asked first gets the outermost. Draw in it with a `nil` pane, at the coordinates it gives you.

It is `nil` in three cases, and all three mean *draw nothing this tick*: you reserved nothing,
your reservation was refused, or the terminal is too small to spare the cells. That last one is
the same rule that hides a pane too small to show — the bands come straight back when the
terminal grows.

The terminal is judged **per axis**. A terminal too short for the rows still affords the
columns: the horizontal bands go and the vertical ones stay, because rows and columns are
spent out of different room and hiding a sidebar over a shortage of rows would give the panes
nothing back for it.

`reserve` takes effect on the **next** frame, like `pane.split`: the frame you call it in was
already composed against the surface it had. Your first band arrives a frame later.

`handle:resize_to(cells)` changes how many cells the band takes, in the edge's own unit,
from the next frame. It is not `resize`, because `neotmux.pane.resize` moves a boundary by a
delta and this sets a size; one verb for both is a bug waiting to be typed.

**A band that resizes keeps its place on its edge.** That is the reason the call exists: bands
sit in the order they were reserved, so a plugin that removed its band and asked for a new one
would come back *inside* everyone who reserved after it, and folding something would walk it
across the screen. Resizing edits the reservation in place, so a band that shrinks to one cell
and grows back is exactly where it was.

It answers `true`, or `nil` plus a message when the cells belong to somebody else — the same
distinction `reserve` draws, and the cap is counted against the *other* bands on the edge, so
a band never pays for its own cells twice. Zero and a count past the edge's bound raise, just
as they do on `reserve`: there is no band of no cells, and `ctx:reserved()` therefore never
answers a rect you must not draw in. Fold to one cell instead — one row or one column is a
rail you can still see.

A horizontal edge holds at most **4 rows** in total, a vertical one at most **40 columns**.
The two numbers are decided separately: four is where a line or two of text stops being a
line of text, while forty is half of the eighty columns a terminal is still written against —
past that a band is not a margin on the panes but the layout. Asking for more than the edge's
own bound, for zero, or for a name no edge has *raises* — those are typos. Losing the cells to
a plugin that asked first returns `nil` plus a message instead, exactly as `keymap.set` does
when it loses a key: you wrote a legal call and lost a race.

### Panes

```lua
neotmux.pane.list()      -- { id, ... }, in layout order
neotmux.pane.current()   -- id, or nil
neotmux.pane.rect(id)    -- { x = , y = , w = , h = }, or nil
neotmux.pane.info(id)    -- { command = ..., cwd = ... }, or nil
neotmux.pane.revision(id)     -- a number that moves when the pane is fed, or nil
neotmux.pane.content(id, opts) -- { line, ... }, revision, wrapped — or nil, nil, nil
neotmux.pane.history(id)      -- how many lines are above the screen, or nil
neotmux.pane.scrollback(id, opts) -- { line, ... }, revision, wrapped — or nil, nil, nil
neotmux.pane.split('horizontal' | 'vertical')
neotmux.pane.focus(id)
neotmux.pane.resize('left' | 'right' | 'up' | 'down', cells)
```

**An id is an identity, not a number to show anybody.** A `PaneId` is handed out once and
never reused, so the id of a pane outlives the pane and the next one gets a fresh one: open
and close a second pane three times and the third of them is id 4, with only two panes on
screen. That is deliberate — a plugin holding id 2 must never find that it now means a
different pane — and it is why nothing renumbers.

What a person reads off a bar is a **position**: where the pane sits among the ones that exist
now, which closes the gap when a neighbour goes. `list()` is in layout order, so the position
is the index in it, and finding it is a loop over the list you already have. The shipped bar
does exactly that in `status/position.lua`. Use an id for anything you look a pane up by, and
a position for anything you print.

`rect` answers out of the same published geometry `ctx:pane_rect` reads, in the same surface
coordinates and with the same table — so geometry is a question you can ask from a key
handler, where there is no `ctx`. Which pane is to my left is that kind of question, and
answering it should not oblige you to draw. Inside a tick `ctx:pane_rect` remains the natural
call. An id no pane has answers `nil`, the way `pane.current()` answers nothing.

`info` is what a status line needs to say more than how many panes there are. `command` is the
pane's **foreground** process, not the shell neotmux spawned for it: a pane sitting at its
prompt reports `bash`, and the same pane running `nvim` reports `nvim`. `cwd` is that same
process's working directory, so it follows a `cd` in the pane.

One call rather than two, because both facts come from the one process the host looked at and
two calls could answer about two different ones. An id no pane has answers `nil`, the way
`pane.current()` answers nothing; a pane whose fact could not be read — a child that exited
between the sample and the question — is a table with that field absent. Neither raises. A
status line must not start erroring because a shell went away.

`info` answers for **every** pane the session holds, hidden windows included, so a pane you
find through `window.panes()` describes itself the same way one in front of you does. The
facts are sampled on the host's own cadence rather than when you ask, and that cadence is what
bounds the cost — asking about ten panes costs no more `/proc` than asking about one.

Ask as often as you like. The facts are sampled by the host on its own cadence, at most every
250 ms, and a call reads the last sample rather than the operating system.

This is read from `/proc` and is therefore Linux-only. On any other platform both facts are
simply unknown.

#### What a pane is showing

`content` is the pane's visible screen as lines:

```lua
local lines = neotmux.pane.content(id)                 -- every visible row
local tail  = neotmux.pane.content(id, { lines = 5 })  -- the last five rows
```

Lines, never bytes. The host has already reconstructed the terminal, so you are handed text
with no escape sequences in it and can match a pattern against it directly. One string per
row, top to bottom; a wide character appears once, at the column it starts in; a cell holding
nothing reads as a space, and the trailing run of them is cut, so an untouched row is `''`. A
cell that was never written and one that was erased are the same cell to the terminal, and
nothing tells them apart.

It is the **visible** screen. What has scrolled off the top is `scrollback`'s question, below.

A pane in a hidden window answers like any other. Its shell is running and its output is being
read into its terminal whether or not you are looking at it, which is the whole point: the
pane you are not watching is the one you want to be told about. `pane.list()` still answers
about the current window only, so a plugin that wants to read a hidden pane finds the id with
`window.panes(id)` over `window.list()`, and forgets it on `pane_closed`. A `pane_output`
event would find it too — every pane's output is dispatched, a hidden window's included — but
only once the pane has said something, and a pane that has been silent since it opened is the
one worth finding.

An id no pane has answers `nil`, and so does a pane that closed between one call and the next.
Neither raises: a loop that reads every pane must not start erroring because a shell exited.

`content` returns the lines and the revision they were read at. `revision` on its own is the
cheap half:

```lua
local rev = neotmux.pane.revision(id)
if rev ~= seen[id] then
  seen[id] = rev
  -- ... read the lines
end
```

Nothing is built until you call `content`, and `revision` is a number out of a map. That is
the arrangement that lets a plugin ask about every pane on every tick without putting every
pane's screen on every frame's bill. The revision says the pane's terminal **has been fed**,
not that the screen looks different — a chunk that only moved the cursor moves it too. A
resize moves it as well, because reflowing changes what the pane reads as.

`opts.lines` bounds the work as well as the answer: the rows above it are never built. Ask for
the few at the bottom if a prompt is what you are after.

`opts.pad` keeps the blank rest of every row instead of cutting it, so each line comes back
exactly as wide as the pane — in **columns**, so a row holding a wide character is two
characters shorter than it is wide. Cutting is right for reading text and wrong for a
rectangle: a selection three columns wide over a line of two characters has to know that the
third column is a space the pane is showing. It is off unless you ask for it, and
`pane.scrollback` takes it on the same terms.

```lua
local rows = neotmux.pane.content(id, { pad = true })
```

`opts.wrapped` adds a third return value: one boolean per line, saying whether that line ran
off the end of the pane and continues on the one below. It is the terminal's own flag, not a
guess — a line exactly as wide as the pane looks identical either way, so there is nothing to
guess from. It is `nil` unless you ask, and `nil` means "not asked", never "none of them".

```lua
local lines, _, wrapped = neotmux.pane.content(id, { wrapped = true })
```

That is what a copy of a wrapped command line turns on: a wrapped line keeps its trailing
blanks and gets no newline after it, an unwrapped one gets both. `pane.scrollback` answers
the same way, which is where the top of a wrapped command line usually is.

A resize clears the flags. The terminal drops them on every row when the column count
changes, because the old wrap points no longer describe the new width, and neotmux does not
invent replacements.

#### What a pane has already said

`scrollback` is the lines above the visible screen, and `history` is how many of them there
are:

```lua
local n = neotmux.pane.history(id)                              -- 0 if it has not scrolled
local all  = neotmux.pane.scrollback(id)                        -- the whole history
local tail = neotmux.pane.scrollback(id, { from = n - 10, lines = 10 })  -- the last ten
```

Same lines as `content`, on exactly the same terms: no escape sequences, one string per row,
a wide character once at the column it starts in, trailing blanks cut. An id no pane has
answers `nil, nil`, and so does a pane that closed between two calls.

**Line 0 is the oldest line the pane still holds**, and numbers increase downwards, so the
line directly above the top of the visible screen is `history() - 1`. The visible screen
itself is not addressable here — it is `content()`.

**A line's number does not change when the child prints more.** New output is appended to the
end of the history and takes the next number; `history()` grows and nothing already numbered
is renumbered. That is the whole reason the numbering runs from the oldest line rather than
from the bottom of the screen: a position you saved last tick still means the line you saved
it for.

**The exception is the history filling up.** A pane keeps 10 000 lines. Once it holds that
many, each new line drops the oldest and every number shifts down by one — so a plugin
holding a position across that will find it pointing one line later for each line dropped.
`history()` sitting still at the limit while the pane is producing output is what that looks
like.

**Reading does not scroll the pane.** The terminal's own position in its scrollback is moved
to reach the lines and put back before you are answered, inside one call, so nobody watching
the pane sees it move.

**The revision gates this too.** `scrollback` returns the revision it was read at, the same
number `content` and `revision` answer with, and the history cannot change without the pane's
terminal being fed — so an unchanged revision means an unchanged history. `history()` is the
cheap question: it is a number, with no line built to answer it.

A read costs the lines it returns: every row it hands back is built, character by character,
out of the terminal's cells. `opts.lines` is what keeps that bill down, and a call with no count at
all is the whole history — up to 10 000 rows, which is not a
thing to do on every tick.

`resize` **moves the boundary between the focused pane and its neighbour in that direction**,
by exactly that many cells — so the pane grows by them and the neighbour loses them. There is
no second rule for a pane at the edge: with no neighbour that way there is no boundary, and
the call is refused rather than quietly moving the pane's other edge. The boundary moved is
the nearest one, so a pane nested three splits deep resizes against what it is actually
touching.

Cells, not a fraction and not a percentage: after a `resize('right', 3)` the pane's rect is
three columns wider, and thirty of them in a row move thirty columns.

A direction nobody can spell and a count that is not a positive number of cells *raise* —
they are typos. A refusal — no neighbour, or a pane that would end up below the minimum grid,
counting the gutter — is logged against you and changes nothing. It cannot be returned to you
the way `keymap.set` returns a lost race: the op is applied after the tick, long after your
call returned.

`split` and `focus` do not take effect while your callback is running. They are queued and
applied after the tick, so `list()` returns the same thing immediately after a `split` as
immediately before. This is deliberate: a callback that could reshape the world underneath the
callbacks running after it would make frame composition depend on dispatch order.

A frame forks at most three new shells, counting `pane.split` and `window.create` together.
Each one costs the host a fork, an exec and a terminal, none of it inside the tick your budget
was measured against, so the number of them one frame can be asked for is bounded; the fourth
and beyond are refused and logged against you. There is no way to close a pane from Lua in v0.1.

`pane_opened`, `pane_closed` and `focus_changed` are dispatched against the layout as it is
after the change, so a handler's `pane.list()` and `pane.current()` describe the event it is
being told about.

A terminal too small to hold the tree hides the panes that no longer fit rather than closing
or shrinking them, and hiding a pane does not unfocus it: keys still go to it, so you are
typing blind until the terminal grows back. The terminal cursor is hidden for as long as that
lasts. It has nowhere honest to sit — every other visible cell belongs to a pane that is not
getting the keys, or to no pane at all — and no cursor is a better answer than one pointing at
the wrong pane.

### Windows

```lua
neotmux.window.list()     -- { id, ... }, in creation order
neotmux.window.panes(id)  -- { pane, ... }, in layout order, or nil
neotmux.window.current()  -- id
neotmux.window.create()   -- the new window's id
neotmux.window.focus(id)  -- show that one
```

A `WindowId` is stable and never reused, exactly as a `PaneId` is, and the same rule follows:
show the position in `list()`, not the id, or your window numbers climb for ever as windows
are opened and closed.

A window is an independent pane tree. Switching between them is what a multiplexer is mostly
for, and it is the only level of grouping above a pane there is: tmux's session, the thing you
detach from, is not built. That is why this level is called a window and not something else —
when a session does arrive it goes *above* windows, and nothing here changes meaning.

`pane.list()` and `pane.current()` answer about the **current** window: a pane in another
window is not in the list, even though its shell is running and its output is being read into
its own terminal, and switching to it shows everything that happened while you were away.
`pane.content()` and `pane.info()` are the exceptions — they answer about any live pane,
hidden or not, because what a pane you are not looking at is showing, and what it is running,
are exactly the questions worth asking.

`window.panes(id)` is how you find those panes in the first place. It answers for any window,
hidden or not, in layout order — left to right, top to bottom, the order `pane.list()` gives
for the window being shown, and the order a hidden window will be shown in. A window id
nothing holds answers `nil`, the way `pane.current()` and `pane.rect()` answer nothing; a
living window always has at least one pane, so an empty list is not an answer you can get.
Finding is all it is: there is no call that splits, focuses or resizes a pane in a window
nobody is looking at. What a hidden window's panes do not get is a size — they keep
the one they last had, and are resized to the surface on the way back in, so a terminal resize
costs one relayout rather than one per window.

`create` and `focus` are queued and applied after the tick, the same as `pane.split`. `create`
is the one call that answers with something anyway: it hands back the id the new window *will*
have, so

```lua
neotmux.window.focus(neotmux.window.create())
```

opens one and switches to it — two ops, applied in that order. `create()` on its own leaves a
window running out of sight.

`window_opened`, `window_closed` and `window_changed` carry `event.window` and are dispatched
against the session as it is after the change. Closing the last pane of a window closes the
window and moves you to another; closing the last window exits neotmux.

A reserved band belongs to the surface, not to a window: a status line reserves its row once
and keeps it across every switch.

### Everything else

```lua
neotmux.log.info(text)    -- also .warn, .error
neotmux.notify(level, text)  -- 'info' | 'warn' | 'error'; shown on screen and logged
neotmux.quit()
neotmux.opt.tick_budget_ms = 8     -- per source per frame; 0 turns the budget off
neotmux.opt.tick_overrun_limit = 5 -- consecutive overruns before the tick is dropped; >= 1
neotmux.opt.pane_gutter = 0        -- cells left between panes, for whoever draws there; <= 4
neotmux.resubscribe(name)          -- give a dropped source its tick back; returns handles
neotmux.width(text)                -- how many columns `text` takes on a pane's grid
neotmux.home                       -- `$HOME` as the host read it, or nil; read-only
```

`neotmux.width` is the one measurement everything that takes a column agrees on. `#text` is
bytes and `utf8.len(text)` is characters, and neither is a column: a wide character is one
entry in a `pane.content` line and **two** cells on the grid, so `#line:sub(1, n)` is not
column `n` on any line that has one in it, and a tint computed that way drifts right by one
cell per wide character before it.

```lua
local line = neotmux.pane.content(id)[1]
ctx:tint{ pane = id, x = 0, y = 0, w = neotmux.width(line), h = 1, bg = 4 }
```

It is the renderer's own rule, read out of the same code the grid is built with, so the two
cannot disagree. A combining mark adds nothing, because it is drawn into the cell before it.
A control character adds nothing either, because it is never drawn — a tab moves the cursor
rather than filling cells, and no tab ever reaches a `pane.content` line for that reason.

`neotmux.home` is a fact, not an option: it sits beside `pane.info`'s facts rather than under
`opt`, because nothing sets it. The VM has no `os.getenv` — reading the process environment is
the host's alone — so this is the one value out of it a plugin ever sees, and it is the same
read the `~/.config/neotmux` layer is resolved from. It is `nil`, not `''`, where the process
had no `HOME`.

Logs go to `$XDG_STATE_HOME/neotmux/neotmux.log`, or wherever `--log` says. Notifications also
replay on stderr when neotmux exits, so a message from a frame you never saw is not lost.

## How your code is identified

Every registration and every API call is attributed to **the chunk the calling code was written
in** — not where it was called from. A function defined in `lua/plugin/clock.lua` is attributed
to `plugin/clock` whether it runs during load, from a tick, from a coroutine, or through
`pcall`. This is what makes the log useful:

```
2026-09-20 10:11:12Z [ERROR] plugin/clock: runtime error: .../clock.lua:4: attempt to ...
```

A name with a slash is a file that was found. A failed `require` is named after the module you
asked for instead — `plugin.nosuch`, with a dot — because there is no file to name.

Drawing from several sources composes in load order, so a plugin loaded later draws over one
loaded earlier — and two plugins reserving the same edge are given their bands in that order
too, outermost first.

One consequence worth knowing: if your plugin gets a callback from a library and registers it,
the **library** owns it, because that is where it was written. Errors are reported against the
library, and a rollback of your plugin will not remove it.

The tick budget inherits this, and it is worth spelling out. Write

```lua
neotmux.on_tick(require('lib.helper').tick)
```

and the callback was written in `lib/helper`, so `lib/helper` is what the log blames and
`lib/helper` is what gets parked after five overruns — **including the ticks every other
plugin registered from that same library**, because they are one source sharing one slice.
This is the model working as intended: the slow code is the library's, and that is where a fix
belongs. It is still a surprise the first time. If you want your own name on it, write your own
function and call the library from inside it.

## When things break

- **A tick raises.** That source's drawing for the frame is thrown away and its previous frame
  is shown again — as long as the window has not changed shape since, because a frame drawn
  for a different size or pane layout is dropped rather than shown stale. Anything the tick
  queued before it raised goes with the frame: a `split`, a `notify`, a `quit` is part of the
  frame you lost. Other sources are unaffected. The error is logged with your name.
- **Two tick callbacks from one file, one of them raises.** Both are thrown away — your frame
  is the union of what your callbacks drew, and half of it is not a frame.
- **A key callback raises.** It is logged and treated as not consuming, so a broken handler
  cannot swallow your keyboard.
- **`require` fails in `init.lua`.** That entry is skipped, anything it registered before
  failing is removed, and the rest of your config still loads. A `require` *inside* a plugin
  raises normally — your plugin's missing dependency is your plugin's error.
- **Your tick runs too long.** It is cut off, you lose the frame, and after five in a row you
  lose the subscription. See the next section.
- **You remove your last tick registration.** Your last frame goes with it. Drawing requires a
  tick; without one there is nothing to refresh what is on screen, so it is cleared rather than
  frozen.

## The tick budget

Your tick gets **8 ms per frame**, and it is 8 ms for the *file*, not for each callback: two
`on_tick` callbacks written in the same file share one slice.

What an overrun looks like from inside your plugin: nothing you can catch usefully. Your code
stops between two instructions with an error you did not raise, wherever it happened to be.
A coroutine does not help — the measurement follows into `coroutine.wrap`. In the log:

```
2026-09-20 10:11:12Z [ERROR] plugin/clock: tick exceeded its 8 ms budget
```

What you lose is **that frame's drawing, all of it** — including what your other callbacks
already drew, because your frame is their union — and everything else that tick queued: a
`neotmux.pane.split`, a `neotmux.notify`, a `neotmux.quit`. A frame the host throws away does
not get to act on the world either. Your previous frame is shown again, unless the window has
resized or its panes have changed since, in which case that frame is dropped instead of shown
at the wrong size. Other sources are untouched, and your keymaps and event handlers keep
working.

After **5 consecutive** overruns your tick subscription is removed and a notification names
you. Now nothing refreshes your drawing, so it is cleared rather than frozen. One tick that
finishes inside the budget wipes the count, so this only happens to a plugin that is over
every single frame.

### Getting back

```lua
local handles = neotmux.resubscribe('plugin/clock')   -- handles, or nil if nothing was parked
```

You get back one handle per tick it restored — the same handle `on_tick` returns — and `nil`
when there was nothing to restore. A restore always brings back at least one registration, so
the list is never empty and `nil` cannot be mistaken for one. Keep the handles: a restored
registration is a *new* one, and the handle you were holding before the park names an id that
no longer exists, so `handle:remove()` on the old one does nothing.

The name is the one the notification used. It is a display name, and two runtime layers that
provide the same relative path share one — if that happens, the earlier layer's copy is the one
restored, and the handles you get back do not tell you which of the two came back.

You get a full 5 frames again, so a plugin that has not been fixed is dropped again.

Re-`require`ing the module instead drops what was parked, so you do not end up subscribed
twice — *unless* you have already called `resubscribe`. What it drops is what is still parked,
and a restored registration is live, so re-`require`ing after a `resubscribe` leaves you with
both. Remove the handles first.

### Turning it off

```lua
neotmux.opt.tick_budget_ms = 0
```

Nothing is measured and nothing is counted — no hook is armed at all. This is what you want
while stepping through a plugin with a debugger, and it means a `while true do end` freezes
neotmux with no way out but killing it.

### What the budget cannot cut off

The hook that measures you fires between Lua instructions, so it cannot interrupt anything that
is not Lua instructions. It still checks the clock when your tick returns, so these are still
your overruns: you lose the frame and you are named in the log. They count towards the five
only past **twice** the budget, because that measurement is wall clock over the whole frame —
it includes the host's own work and a garbage collection step some other plugin triggered — and
a plugin that is a millisecond over on a loaded machine has not done anything wrong:

- **one long C call.** `string.rep(s, 1e8)`, a huge `table.concat`. Your code is not stopped —
  there is no way to stop it mid-call — so the whole call runs and the rest of neotmux waits
  for it. Measured: a tick doing one 120 MB `string.rep` blocked the frame for 189 ms against a
  5 ms budget. The log says what was seen, not where the time went: `tick exceeded its 5 ms
  budget: 189.4 ms, not cut off — no sample landed past the deadline`
- **`__gc` finalizers.** Lua switches the hook off while a finalizer runs, so the same applies.
  Measured: 162 ms in one finalizer
- **`pcall`.** The budget error is an ordinary Lua error, so `pcall` catches it. It buys you
  nothing: you still lose the frame and the overrun is still counted against your limit. An
  unbounded `while true do pcall(f) end` does hang neotmux
- **a host binding that blocks.** That would be our bug, not yours

None of this is a sandbox. Plugins are trusted (below); the budget is there so an *accident*
costs one plugin one frame instead of costing you the program.

## The shipped defaults

neotmux ships five plugins in the runtime layer an install puts on the search path:

| plugin | what it does |
|---|---|
| `status` | the default status bar, on one reserved top row |
| `prefix` | a `C-b` prefix over `v`, `-`, `o`, `h/j/k/l`, `r`, `c` and `n` |
| `borders` | a rule along every boundary between two panes |
| `watch` | says which pane looks like it is waiting for you — **off by default** |
| `sidebar` | the windows and their panes, down a reserved left band, folded by `C-b b` — **off by default** |

None of them is a host feature — all five are ordinary files making ordinary API calls, and
the prefix is the only reason a fresh install can make a second pane at all. `C-b v` splits
vertically (the panes end up side by side, because `pane.split('vertical')` divides the
width), `C-b -` splits horizontally (one above the other), `C-b o` focuses the next pane,
`C-b h/j/k/l` focuses the pane to the left, below, above or to the right, `C-b r` enters
resize mode, `C-b c` opens a window and switches to it, and `C-b n` walks to the next window.
With the sidebar on, `C-b b` folds it — the sidebar adds that one itself, since it is the
optional plugin and the prefix is the standard one.

`h/j/k/l` are geometry, not list order: the pane you get is the one whose edge faces most of
yours on that side, which is what you would point at however deeply the splits are nested.
There is no wrapping — at the edge of the window the key does nothing, and `o` is still there
for walking every pane in turn.

Resize mode is a flag in the plugin and an `on_key`, like the prefix itself; the host has no
idea it exists. In it, the arrows and `h/j/k/l` move the boundary one cell at a time
(`require('plugin.prefix').resize_step` sets how far), and the first key that is not one of
them leaves — `Esc` and `Enter` being the two that mean nothing else.

### Turning one off

Every layer's `init.lua` runs, earliest layer first, and the shipped layer is last — so your
own `init.lua` can decide what the shipped ones do:

```lua
-- ~/.config/neotmux/init.lua
require('neotmux.defaults').status = false
require('neotmux.defaults').prefix = false
require('neotmux.defaults').borders = false
```

`watch` and `sidebar` go the other way: they are off until you turn them on. What counts as a
pane waiting for you is an opinion about what you run, and twenty-four columns of sidebar is
an opinion about screen real estate; a status line is neither.

The sidebar has two widths rather than an on and an off — `require('plugin.sidebar').widths`
is `{ open = 24, closed = 1 }`, and `C-b b` swaps them. Closed is the markers column: you can
still see how many windows and panes there are and which one you are in, you just cannot read
their names. A band of nothing would be a band you have to remember is there.

```lua
require('neotmux.defaults').watch = true
require('neotmux.defaults').sidebar = true
```

That is a Lua module the runtime layer ships, not a host feature: neotmux has no idea the
defaults table exists. Deleting the installed `lua/plugin/` directory works too, and leaves a
multiplexer with no status line, no prefix, no borders and no error — and with every row of
the terminal going to the panes, because nothing is left to reserve one.

### Changing what the prefix does

`plugin.prefix` returns its key and its command table, and reads both when a key arrives, so
assigning to either from your own `init.lua` takes effect:

```lua
-- ~/.config/neotmux/init.lua
local prefix = require('plugin.prefix')
prefix.key = 'C-a'                            -- rebind the prefix itself
prefix.commands['%'] = prefix.commands['v']   -- tmux's muscle memory, alongside v and -
prefix.commands['"'] = prefix.commands['-']
prefix.commands['o'] = nil                    -- and one taken away
prefix.commands['!'] = function()             -- and one of your own
  neotmux.notify('info', 'panes: ' .. #neotmux.pane.list())
end
```

A command is a function of no arguments, keyed by the key spec that runs it — the same specs
`keymap.set` takes. The prefix swallows the key after itself whether or not the table has an
entry for it, so a mistyped command never leaks a character into the shell.

`require`ing it from your `init.lua` loads it there instead of later, which is fine: the
shipped layer's `require` finds it already loaded. Do not do it *and* set
`defaults.prefix = false` — the module is loaded either way, and the flag then only looks
like it turned something off.

### Changing what the status bar looks like

`plugin.status` reserves the top row and draws the windows on the left, then the focused
pane's command and its working directory on the right. A window's block is numbered with its
position, and the one being shown carries the focused pane's position after a dot — tmux's
`window.pane`. Positions and not ids, for the reason the pane API gives. It returns four
tables, all read every tick:

```lua
local status = require('plugin.status')
status.colors.current = '#f5c2e7'   -- the cap on the window being shown
status.caps = { left = '', right = '' }              -- no Nerd Font: no caps
status.icons = { command = '', directory = '' }      -- and no icons
status.home = '/home/you'           -- the prefix written `~`
```

The glyphs default to Nerd Font ones — a rounded cap either side (`U+E0B6`, `U+E0B4`), and
one in front of each module (`U+F1AE`, `U+F07B`). Without such a font they come out as boxes,
and the two lines above are the whole escape: empty strings leave the coloured blocks and take
the glyphs off.

`colors` is Catppuccin Mocha, keyed by what each colour is for rather than by its name:
`bar`, `current`, `inactive`, `on_current`, `on_inactive`, `body`, `body_text`, `module`,
`module_text`, `icon`, `command`, `directory`.

`home` is what gets written `~`, the way a shell prompt writes it. It defaults to
`neotmux.home`: the VM has no `os.getenv`, so the host reads `$HOME` once — the same read the
config layer is resolved from — and publishes it. It is `nil` where the process had no `HOME`,
which costs you the `~` and nothing else; assign to it if you want a different prefix.

A directory too long for the space left loses its leading components — `.../src/neotmux` —
and is dropped along with its icon if not even its last component fits.

### Changing what the sidebar looks like

`plugin.sidebar` takes twenty-four columns off the left and lists the session down them: a row
per window, then a row per pane of that window with what it is running. A ▸ marks the
window being shown and a • the focused pane. Numbers are positions, the same way the
status bar writes them, so a pane reads `2.1`.

```lua
local sidebar = require('plugin.sidebar')
sidebar.colors.focused_pane = '#f9e2af'
sidebar.markers = { current_window = '>', window = ' ', focused_pane = '*', pane = ' ' }
```

`colors` and `markers` are read every tick. A marker is one cell: one that measured two
columns would push every name on its row across by one.

The **width** is not in that table. It is a `local` at the top of the file, because the
reservation is made once while the file is being required and the host is never told a band
changed width. Edit it there, or leave the sidebar off.

### Changing what the borders look like

`plugin.borders` returns `{ fg = 8, vertical = '│', horizontal = '─', cross = '┼', tee_up = '┴',
tee_down = '┬', tee_left = '┤', tee_right = '├' }`, all read every tick:

```lua
local borders = require('plugin.borders')
borders.fg = '#404040'
borders.vertical = '|'      -- ASCII, if your font or terminal prefers it
borders.horizontal = '-'
borders.cross = '+'
```

Each glyph is one character, because a divider is one cell: a terminal cell cannot be
subdivided, so box drawing is as thin as a line gets.

The last five are what goes in a cell two dividers meet in. Which one is not a choice the
plugin makes per layout: `neotmux.junctions` is handed the segments and answers which of the
four arms are there, and a tee is named for the arm that is on its own — `├` is `tee_right`,
because right is the way its stem points. A junction with fewer than three arms is a line
carrying on, and nothing is drawn over it.

The plugin sets `neotmux.opt.pane_gutter = 1` and draws in the reserved column or row, so a
border costs no pane a cell of its shell's output. Switch the plugin off and the gutter goes
with it — the panes tile the surface again.

## Reloading

There is no reload function. There are two lines of Lua:

```lua
package.loaded['plugin.clock'] = nil
require('plugin.clock')
```

Note what this does *not* do: your old registrations are still there. (The exception is a tick
the budget is still holding for you: registering a tick again drops it. A tick you already got
back with `resubscribe` is live, not held, so that one does not go.) Keep the handles and
remove them first if re-running the chunk would register twice.

## Three that work

`examples/` has three plugins written only against what is documented here, each showing
something the others do not:

- `clock.lua` — reserving a row and drawing in it, and a table a user assigns to from their own config
- `prefix.lua` — `on_key`, modal state, and driving panes; a tmux-style prefix that binds no key
- `pane_labels.lua` — drawing into a particular pane, so a label follows it through splits

Copy one into `~/.config/neotmux/lua/plugin/` and add its `require` line. Delete the line and
it is gone.

## What the sandbox is not

Plugins are trusted. They run in-process with no isolation, and the trust model is "you
installed it". What is removed is removed because it would hang or crash the program, not to
contain a hostile plugin: `io` and `os.execute` block, `dofile`/`loadfile`/bytecode chunks can
crash the interpreter, and `debug` can reach anything. `load` works and gets the real globals.

Do not install a plugin you would not run as a shell script.

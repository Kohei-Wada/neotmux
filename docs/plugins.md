# Writing a neotmux plugin

neotmux has no plugin system. It has an API, and a convention for where Lua files live. A
plugin is a file that calls the API; it is a plugin only because you call it one. Nothing
registers it, nothing declares it, and the program never learns the word.

That is not a slogan — it is the reason things work the way they do below. There is no manifest
because there is nothing to declare to. There is no enable/disable because loading a file is
what "enabled" means. There is no plugin name because the thing neotmux knows about is the
*chunk your code was written in*, which it names after the file's path.

## Where your code runs

Your plugin runs in the neotmux **server** — the process that holds the panes, the shells and
the terminal state — and there is one Lua layer for a session, shared by every client attached
to it. Two terminals looking at the same session are looking at the same plugins, with the
same modes, the same floats and the same reservations.

It keeps running while you are detached. That is the whole point: a plugin that watches a
long-running job, answers it, or starts the next one when it finishes is doing its work in the
window where nobody is looking, and it can only do that if it is running. `on_tick` ticks with
no terminal attached, exactly as it does with one.

What that costs is that a plugin which wedges wedges the session rather than one terminal,
which is what the tick budget below is for.

It also means **nothing on this API waits**. Anything that has to wait for something to happen
is a subscription that delivers on a later tick, never a call that returns when the waiting is
over. That is a contract and not an implementation note: the budget interrupts Lua between
instructions, and no Lua instruction runs while the host is inside a blocking call, so a host
call that waited could not be cut off.

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
| `neotmux.on_write(fn)` | before text is written into a pane; return `false, why` to refuse it |
| `neotmux.float({ x =, y =, w =, h = })` | nothing runs; the grant is two-valued — a handle, or `nil` plus a message (below) |
| `neotmux.keymap.set(spec, fn)` | for that one key; return `true` to consume it |
| `neotmux.reserve(edge, cells)` | nothing runs; it takes a band of the surface (below) |
| `neotmux.command.add(name, fn, { desc = })` | when `neotmux.command.run(name)` names it (below) |

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

### Commands

A name a plugin registers once, that anything — a key, another plugin, the palette below —
can call by it afterward:

```lua
neotmux.command.add('scratch', function(args)
  neotmux.window.focus(neotmux.window.create())
end, { desc = 'open a scratch window' })

neotmux.command.list()          --> { { name =, desc =, source = }, ... }, in name order
local ok, why = neotmux.command.run('scratch', { 'arg' })
```

`add` answers `true`, or `nil, message` when the name is already held — the first registration
wins, so a config layer's own `init.lua` (which runs first) can claim a name before the shipped
plugin that would otherwise take it does. `run` takes the name and an array of strings, and
answers `ok, err, lines`: `false` and a message on a name nobody registered, on a handler that
raised, or on a handler whose return was not one of the three shapes below; `true`, `nil` and
the lines otherwise. A handler answers `nil`, a string, or an array of strings, which become no
lines, one line, or that many — anything else is a failure against the plugin that registered
it, the same as a handler that raises.

It is shipped as `command.add`, not a callable `neotmux.command(...)`: every top-level table in
this ABI is sealed, and a callable table is a metatable the seal would have to make an
exception for. `keymap.set` is the shape this ABI already uses for "register a named thing",
and `command.add` follows it.

A command is in-process only — nothing outside this VM can name one yet. What it buys you today
is the palette below, and a plugin's own verbs being one definition two ways in: see
[Changing what the prefix does](#changing-what-the-prefix-does), where `plugin.prefix` registers
each of its verbs this way and its key table calls them by name.

### Drawing

Drawing happens only from a tick, through the `ctx` your callback is handed. It is
immediate-mode: **what you draw this tick is your entire output for this frame.** Draw nothing
and you show nothing. There is no clear call because you never need one.

```lua
neotmux.on_tick(function(ctx)
  local size = ctx:size()                      -- { rows = , cols = }: what the panes have
  local area = ctx:area()                      -- { x = , y = , w = , h = }: the same, placed
  ctx:text(nil, x, y, 'text', { fg = 2 })      -- nil pane = the whole surface
  ctx:rect(nil, x, y, w, h, { bg = '#202020' })
  ctx:tint(nil, x, y, w, h, { bg = 4 })        -- recolours, writes no glyph
  ctx:line(nil, x1, y1, x2, y2, { fg = 4 }, '|') -- no bg: it paints single cells
                                               -- the glyph is optional and defaults to '#'
  ctx:text(win, x, y, 'text')               -- win = a float: its own coordinates, not the surface's
  local r = ctx:pane_rect(pane_id)             -- { x = , y = , w = , h = } or nil
  local band = ctx:reserved()                  -- your own band, or nil (below)
  local float = ctx:float_rect(win)           -- your own float, or nil (below)
  ctx:view(pane, 12)                          -- draw that pane 12 lines back in its own history
end)
```

Colours are a palette index `0..255` or a `"#rrggbb"` string. Omit the table and you get the
terminal's defaults.

`ctx:line`'s glyph is one character: it fills one cell per step, so anything longer raises
rather than being truncated.

`ctx:size()` is the area available to the shells, so it shrinks by whatever is reserved at the
edges. Read it every tick rather than caching it. `ctx:area()` is that same area with the
corner it starts at: a band reserved at the left moves the pane area across without changing
its size, so anything anchored to an edge — a box in the bottom-right corner, say — is drawn
from `area.x + area.w`, never from `size.cols`.

**Coordinates are the surface's.** A `nil` pane means the whole terminal, reserved bands
included, and `ctx:pane_rect` answers in those same coordinates — a band at the top moves the
panes down, so a pane's rect is never relative to anything but the terminal itself. A float is
the one target with its own frame: `ctx:text(win, …)` and the rest of the draw verbs speak in
float-local cells, `(0, 0)` at the float's own top-left, and a command that runs past the rect
is clipped to the float and then to the surface — the float's own bounds, then the terminal,
nothing in between.

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

### Floating a window

```lua
local win, message = neotmux.float({ x = 10, y = 5, w = 40, h = 12 })  -- surface coordinates
win:move_to(20, 6)                                    -- the same float, elsewhere
win:resize_to(50, 20)                                 -- the same float, a different size
win:remove()                                          -- idempotent, like every other handle
```

**A float takes no cells.** The layout, `ctx:size()` and everything computed from them are
untouched — a band is a claim on the surface, a float is a claim on nothing, which is the whole
difference between the two. A float and a band may occupy the same cells: a float is an
overlay, and what is drawn later is what is seen. The server never knows it exists — the panes,
the shells, the layout and the focus are the server's, and a float is none of them.

`move_to` and `resize_to` set the float where it is and how big it is, from the next frame —
the same next-frame delay as the grant itself — and they answer the same two ways the band's
handle does: `true`, or `nil` plus a message. `resize` is not a verb here for the reason the
band's section has: one verb for a delta and one verb for a size is a bug waiting to be
typed.

`win.__id` is the host's own seam — its drawing layer reads it through to resolve
`ctx:text(win, …)` and `ctx:float_rect(win)` — and it is private, not part of the ABI, and
nothing you are meant to name.

`ctx:float_rect(win)` answers **your own** float as `{ x, y, w, h }` in surface coordinates, or
`nil`, and a `nil` means *draw nothing into it this tick*: its draws are dropped.

**The host keeps one order, grant order, and it says the whole of the depth question.** A later
float is drawn over an earlier one for as long as both exist, and the way back is the way every
other reservation comes back: `remove` and a new grant, which makes it the newest. The drawn
layers, bottom to top, are panes, bands, floats in grant order, notices.

The grant takes effect on the **next** frame, like `reserve` and `pane.split`, so it may be made
during `init.lua`: the grant is a pending rectangle until the first frame has a surface to sit
on. A rectangle that stretches past the present surface is granted, not refused: the request is
not mistaken, the surface was merely smaller, and the clipping already described is what keeps
both honest.

**The bargain is the same as `reserve`: the handle persists and the drawing does not.** Every
tick, for as long as the float exists, you draw its whole content into it — immediate mode, and
a float you stop drawing into keeps no content of its own, because it never had any.

**On the keyboard it is one rule: the owners of live floats are offered every key before the
rest of the plugin layer, the topmost float first.** A plugin that holds several floats sits
once, at the position of its topmost one; the rest is the keyboard you already have, and a
float declines a key the way every other callback does — returns nothing, and the key goes on.

**A float dies with its owner, not with the frame.** An `unload` — or the unsubscription an
over-budget plugin earns — takes the float with it, the same fate the owner's bands keep; a
resize does not touch it, the rect stays where it was and only its clipping moves; and a detach
is the owner going, so its floats go with the client, and a re-attach re-runs `init.lua` with
them gone until the plugin that held one asks for it again. The panes, the shells, the session
survive it all, because the float was never any of those.

`neotmux.float` answers `nil` plus a message when the rect is not a rect — a missing key, a
non-integer, a negative `x` or `y`, or a zero size — and records nothing, so a failed grant
leaves the rest of the tick running; `move_to` and `resize_to` answer the same two ways and a
bad answer changes nothing. `remove` does not answer at all, because it has nothing to lose: it
is idempotent, the way every `remove` in this API is.

### Panes

```lua
neotmux.pane.list()      -- { id, ... }, in layout order
neotmux.pane.current()   -- id, or nil
neotmux.pane.rect(id)    -- { x = , y = , w = , h = }, or nil
neotmux.pane.info(id)    -- { command = ..., cwd = ... }, or nil
neotmux.pane.cursor(id)  -- { x = , y = , visible = }, or nil
neotmux.pane.mode(id)    -- { canonical = , echo = , alternate = , application_cursor = }, or nil
neotmux.pane.revision(id)     -- a number that moves when the pane is fed, or nil
neotmux.pane.content(id, opts) -- { line, ... }, revision, wrapped — or nil, nil, nil
neotmux.pane.history(id)      -- how many lines are above the screen, how many fell off
neotmux.pane.scrollback(id, opts) -- { line, ... }, revision, wrapped — or nil, nil, nil
neotmux.pane.freeze(id)       -- hold the pane still; false if there is no such pane
neotmux.pane.unfreeze(id)     -- let it catch up; false if there is no such pane
neotmux.pane.frozen(id)       -- true, false, or nil for no such pane
neotmux.pane.split('horizontal' | 'vertical')
neotmux.pane.focus(id)
neotmux.pane.resize('left' | 'right' | 'up' | 'down', cells)
neotmux.pane.send(id, text)   -- true, or false and why
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

`cursor` and `mode` are what a plugin on the input side needs and could only guess at before:
where the program put the cursor, and what it has told the tty to do with a line as it is
typed. Both are published with the frame rather than asked for, so they cost a plugin nothing
to read every tick and describe the pane as of this frame — the same bargain `revisions` makes.

`canonical` and `echo` are the tty's own `ICANON` and `ECHO`, read from the pty master, which
on Linux answers for the slave without the child being touched. The three states worth telling
apart:

| canonical | echo | what is happening |
| --- | --- | --- |
| true | true | a program with no line editor of its own is reading a line — a plugin may take over |
| true | false | a password prompt — **never touch it** |
| false | false | the program edits for itself (readline, vim, less) |

They are **absent rather than false** where the host could not read them: a plugin that cannot
tell a password prompt from a shell must do nothing rather than something, so write
`if mode.canonical == nil then return end` and mean it. `alternate` says a full-screen program
has the pane; `application_cursor` is DECCKM, which is how an arrow key is encoded when it is
sent.

A cursor is where the *program* put it, which is where typing appears only when the program is
the one reading the line — `mode` is what says whether it is. `visible` is whether the program
is showing it; a hidden cursor is still somewhere. Both answer `nil` for a pane this frame said
nothing about, which is a pane that has closed.

`send` writes text into a pane's shell, as though it had been typed there — the in-process half
of `neotmux send`. The bytes go as written: no key names, and nothing wrapped in a bracketed
paste, because what a program expects is not the host's to guess. A caller that wants either
writes the escapes itself.

Its `true` means **accepted**, not arrived. Like every other request a tick makes, a send is
applied after the tick, so a tick that raises or overruns loses its send with the rest of its
frame — which is also the answer to "does a dropped frame still type at my shell": it does not,
because nothing was written while the tick ran. What can be answered at the moment of the call
is whether this session holds that pane, and `false` is that answer. Any pane the session holds
is reachable, including one in a window nobody is looking at — which is usually the pane a
plugin is answering. A frozen pane takes a send too: `freeze` holds the screen somebody is
reading, not the shell underneath it.

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

#### Holding a pane still

A selection highlight over a pane that is still printing is a highlight over different text a
moment later: the lines move under it and the tint stays where it was put. `freeze` stops
that.

```lua
neotmux.pane.freeze(id)
-- the pane draws, reads and numbers its history as it was at this moment
neotmux.pane.unfreeze(id)
```

While a pane is frozen the host draws the screen as it was when you froze it, and
`pane.content`, `pane.scrollback` and `pane.history` all answer about that same held instant
— **the frozen text, not the live text**. That is the point: a plugin computing a selection
against text the host is not drawing has computed the wrong selection, and being handed the
live text silently would defeat the freeze. `pane.revision` is held with them, so it is still
true that the revision a read comes back with is the revision of what was read, and a watcher
polling it does not re-read the same held text on every frame.

Nothing is lost. The child's output goes on being read from its PTY, parsed, and fed into the
live screen underneath, and `pane_output` goes on being dispatched for every chunk of it —
freezing decides what is drawn and what is read, not whether the child is heard. Unfreezing
shows everything that arrived.

Four things worth knowing:

- **A second `freeze` keeps the snapshot it already has.** It answers `true`, because the pane
  is frozen when it returns, but it does not re-take the view — re-anchoring it would move a
  selection out from under whoever took one. Unfreeze and freeze again if that is what you
  want.
- **A resize reaches the child, not the held view.** The pane's PTY and its live screen are
  resized as usual, because a child must never be lied to about its terminal. The frozen view
  keeps the size it was taken at and is clipped to whatever room the pane now has; unfreezing
  shows the reflowed screen. A selection anchored to a frozen line survives the resize, and
  what it names after the unfreeze is the reflowed text, which is a different thing.
- **Freezing is not a lifeline.** A pane whose child exits closes like any other, frozen or
  not: `pane_closed` is dispatched, and every read about it answers `nil` from then on.
- **It costs what the pane costs.** The held view is a copy of the pane's whole screen,
  scrollback included — a pane with a full 10 000-line history is tens of megabytes of it.
  Freeze the pane you are working in, not every pane.

#### What a pane has already said

`scrollback` is the lines above the visible screen, and `history` is how many of them there
are:

```lua
local n, dropped = neotmux.pane.history(id)                     -- 0, 0 if it has not scrolled
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

**The second number `history()` returns is how many lines have gone that way** since the pane
opened. That is what makes a saved position safe to hold: `dropped + index` is a number
nothing renumbers, so save it and convert back when you use it.

```lua
local _, dropped = neotmux.pane.history(id)
local anchor = dropped + index          -- save this, not the index

local _, now = neotmux.pane.history(id)
if anchor < now then
  -- the line it named is gone; there is nothing to fall back to
else
  local line = neotmux.pane.scrollback(id, { from = anchor - now, lines = 1 })[1]
end
```

The revision will not tell you this: it moves on any output at all, and stands still for none
of the evictions.

**The count can be low, in one case.** The terminal does not report evictions; the host counts
them by watching the lines the terminal appends, and that count is exact as long as no single
read from the pane's PTY appends as many lines as the pane holds. A read is at most 8 KiB and
the pane holds 10 000 lines, so ordinary output — which needs a byte per line — cannot reach
it. A child that sends thousands of scroll sequences in one chunk can, and then `dropped` is
lower than the truth. Everything the plugin could have anchored to is gone in that case
anyway, so what it loses is the numbering rather than the ability to notice.

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

### Guarding a write

```lua
neotmux.on_write(function(write)
  -- write.pane, write.text, write.from ('tool' | 'plugin')
  if write.text:find('rm%s+%-rf') then
    return false, 'rm -rf is blocked'
  end
end)
```

Every write into a pane — `neotmux send` from outside, and a plugin's own `pane.send` — is
offered to every handler registered this way before it happens, in the order they were
registered. Answering nothing, or `true`, passes it. `false`, with or without a reason, refuses
it, and the first refusal is the answer: nothing after it runs.

That is not the same three doors this document keeps open elsewhere. A person's own keystrokes
are `on_key`'s, not this one's — there is no text to hand a handler until it has already gone
through the tty and come back out the other side as pixels, which is why `neotmux.state`
guesses at a screen instead of reading a keystroke. A read is not a write and has nothing here
to answer. And an agent that never calls into neotmux at all — one driving its terminal some
other way — was never at this door to begin with; enforcing panes-only belongs to the agent's
configuration or a sandbox, not to a hook that only ever sees what comes through a pane.

What this protects is writes that come through a pane, and nothing else. It is not a defence
against an agent with write access to the configuration that installs it: a runtime layer is a
file, and an agent that can write files can write one that registers nothing, or edit your
`init.lua` to remove the `require` that loads the guard — and the guard is gone while the pane
still looks the same and nothing announces that it happened. If an agent's filesystem access is
the thing you do not trust, this hook is not what closes that gap.

A handler that raises, or that runs past its budget, refuses the write — the opposite of the
rule the rest of the command surface answers to, where a failing Lua layer lets the built-in
answer instead. This hook inverts that, and only this hook: a guardrail that fails open is
useless exactly when something is wrong, so a broken handler is a closed door rather than an
open one.

With nothing registered, the path a write takes is unchanged: the host asks `has_write_hooks`
before it does anything else, and a session with no handler pays nothing for the question.

#### The `neotmux.guard` convention

`on_write` is one hook, and answering to it in prose gets old after the second rule. `neotmux.guard`
is a pattern table over it, in the same spirit as `neotmux.mode` and `neotmux.box`: the host
has never heard of it, and replacing the file changes nothing about neotmux.

```lua
local guard = require('neotmux.guard')
guard.block('rm%s+%-%w*[rf]', 'rm -rf is blocked')          -- everyone
guard.block('terraform%s+destroy', 'ask first', 'tool')     -- agents only
```

`block` takes a Lua pattern, what to say when it matches, and optionally which writer it is for
— `'tool'` for a connection that is not a client, `'plugin'` for a plugin's own write,
`'terminal'` for a person's own Enter. `guard.only(from)` sets that for every rule added after
it, so a file full of agent rules writes it once.

**`only` is a default for what follows, not a filter over the table**, and the difference
matters because the table has two readers. A layer that narrowed the whole table to `'tool'`
would silently switch off the rules a person's Enter is checked against — one file turning off
another's guard, which is the failure this exists to prevent.

One handler is registered, the first time a rule is added, so a rule added later from your own
`init.lua` needs no new subscription — it is read off the table on the next ask.

`block`'s first argument is a pattern, not the literal it looks like, and the two disagree
about what a character like `-` means. `guard.block('rm -rf', 'blocked')` never matches
`rm -rf /tmp/x`: `-` means "zero or more of the previous item, preferring fewer", so the
pattern reads as "rm, then any number of spaces, then rf" and the actual space before the `-`
in the command breaks it — while the same pattern matches `rmrf`, which nobody meant to name.
A rule that looks armed and silently is not is the worst failure a safety checkpoint can have,
so a literal command is written with `guard.plain`, which escapes what a pattern would
otherwise read specially:

```lua
guard.block(guard.plain('git push --force'), 'no force pushes')
```

A rule can also be a function, for the case a pattern cannot express — `guard.rule(fn)` hands it
`{ pane, text, from }` and takes back a sentence or nothing, so "anything at all typed into a
pane running `ssh prod-…`" is written by reading `neotmux.pane.info(what.pane).command`. The
host has no notion of production and this is where one belongs.

#### The same rules, on the keys you type

`plugin.guard` is the other half, and the reason the rules live in a table rather than inside an
`on_write` handler: what an agent writes is checked before it is written, and what *you* type is
checked when you press Enter. The mechanisms have to differ — a key is not a write, and the host
will never know what is half-typed into a program — so the plugin reads the row the cursor is on
and asks the same `guard.verdict`. A match swallows that Enter, raises the mode `guard` for
whoever draws modes, and says why; pressing Enter again within `plugin.guard.grace` seconds runs
it.

It is **off by default**, and it declines to act in two states, which is what `pane.mode` is for:
a pane reading a line without echoing it is a password prompt, and a pane on the alternate screen
is a full-screen program where Enter is not a command at all. In neither does it read the screen
or take the key.

What it can see is the limit worth knowing: the row as drawn, prompt and all. A prompt with
`rm -rf` in it matches, a command wrapped across two rows is seen in part, and text a program
never echoed is not seen. It is a second pair of eyes, not a proof — unlike the write side, where
the bytes are known exactly.

### Windows

```lua
neotmux.window.list()     -- { id, ... }, in creation order
neotmux.window.panes(id)  -- { pane, ... }, in layout order, or nil
neotmux.window.current()  -- id
neotmux.window.create()   -- the new window's id
neotmux.window.focus(id)  -- show that one
neotmux.window.name(id)   -- what somebody called it, or nil
neotmux.window.rename(id, name)  -- call it that; nil takes the name away again
```

A `WindowId` is stable and never reused, exactly as a `PaneId` is, and the same rule follows:
show the position in `list()`, not the id, or your window numbers climb for ever as windows
are opened and closed.

A name is the thing that does *not* change when another window closes, which is why a window
has one at all. `nil` is a window nobody has named, and its position is what it is called until
then. `rename` is an op like every other: raised in your tick, applied after it.

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

### Sessions

One server holds every session you have started, and one Lua layer serves all of them. So a
plugin can see the others, not only the one whose tick it is running in.

```lua
neotmux.session.current()          -- the session this tick, key or event is for; nil at load
neotmux.session.list()             -- every session there is, in name order
neotmux.session.with(name, fn)     -- run fn against that session, and give back what it returned
neotmux.session.create(name)       -- a new session, at the size of the one that asked
neotmux.session.switch(name)       -- move to it
neotmux.session.kill(name)         -- end it, and every shell in it
neotmux.session.rename(from, to)   -- give it another name
```

`with` is the interesting one, and it is how the shipped sidebar says what each session is
running:

```lua
local panes, command = 0, nil
neotmux.session.with('work', function()
  panes = #neotmux.pane.list()
  local focus = neotmux.pane.current()
  local info = focus and neotmux.pane.info(focus)
  command = info and info.command
end)
```

Every ordinary read works inside it, `pane.content` and the rest included — it rebinds which
session the reads are about, and nothing had to be written per call to make that so. Two things
it does not do:

- **It does not move your drawing.** `ctx` belongs to the session whose tick is running, so a
  `ctx:text` inside a `with` still draws where the tick is. Drawing one session's panes into
  another's surface is not a thing there is an API for.
- **It is not an error to name a session that is not there.** You get `nil` and the function
  does not run: a session can end between one tick and the next, and a watcher's loop is not
  your mistake.

`create`, `switch` and `kill` are ops like `pane.split` — they take effect after your tick, not
during it, so a `create` followed by a `list` in the same tick does not name the new one.
`switch` moves **every client attached to the session that asked**, which with one terminal
attached is the terminal that asked.

### Everything else

```lua
neotmux.log.info(text)    -- also .warn, .error
neotmux.notify(level, text)  -- 'info' | 'warn' | 'error'; raised on the surface and logged
neotmux.notifications.list() -- what is on it now: { level =, tag =, text = }, newest first
neotmux.notifications.clear()-- take them all down; the transcript keeps them
neotmux.quit()
neotmux.detach()          -- leave the session running and let go of this terminal
neotmux.session_name()    -- the session this tick, key or event is for; nil during a load
neotmux.opt.tick_budget_ms = 8     -- per source per frame; 0 turns the budget off entirely
neotmux.opt.tick_overrun_limit = 5 -- consecutive overruns before the tick is dropped; >= 1
neotmux.opt.config_budget_ms = 5000 -- what one layer's init.lua gets, for the layers after it
neotmux.opt.pane_gutter = 0        -- cells left between panes, for whoever draws there; <= 4
neotmux.resubscribe(name)          -- give a dropped source its tick back; returns handles
neotmux.width(text)                -- how many columns `text` takes on a pane's grid
neotmux.clipboard.set(text)        -- written? — true, or false and why not
neotmux.home                       -- `$HOME` as the host read it, or nil; read-only
```

A plugin that wants to behave differently by session branches on it; there is no per-session `init.lua`.

A detach ends the client's conversation, not the session: the panes, the shells and each
pane's scrollback are the server's, and stay. Every plugin's own state is the client's, and
does not — the copy register, a raised mode, a fold, and the reservations, which the plugin
re-makes at load. Re-attaching is a plugin restart, in those words.

**Two clients in one session.** The copy mode one has is not the one the other has,
because the copy mode is the client's and not the session's; the key that one types
and its copy mode does not consume is the key the shell on the other one sees, because
that key is the session's. A band one plugin reserves is the session's surface it
reserves it from, not a row the other client's plugins did not ask for — the last
client to attach is the one the session was told to be, and the one that is not is
the one that looks at it.

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

`neotmux.clipboard.set` sends an OSC 52 sequence carrying `text` to the terminal of every
client attached. Your plugin runs in the server, which has no terminal of its own, so with
nobody attached there is nothing to write to and the call answers `false` with that as the
reason.

```lua
local ok, err = neotmux.clipboard.set(selected)
if not ok then
  neotmux.notify('warn', 'could not reach the terminal: ' .. err)
end
```

**`true` means the sequence was written and flushed, and nothing more.** It does not mean the
clipboard holds the text, and neotmux cannot find out whether it does. OSC 52 has no reply; a
terminal that does not implement it discards it in silence, as it would any escape sequence it
does not know; and a terminal that does implement it may have it turned off, or there may be
an ssh session or another multiplexer in between that dropped it. `false` comes with the
reason, and it is always about the write — there was no client attached to write to, or
writing to one failed.

So do not put "Copied to clipboard" on the screen on a `true`. That is the bug this return
value exists to let you avoid: tmux checks terminfo's `Ms` capability and writes nothing at all
when it is absent, and a plugin that reported success on top of that says "copied" over
nothing having been copied. neotmux does not read terminfo, because `Ms` describes what the
entry for `$TERM` claims rather than what the terminal on the other end did, and because
sending an OSC a terminal does not know costs nothing. Say what you know — "sent to the
terminal" — or say nothing, and tell the person about the paste buffer you keep yourself,
which you can actually check.

There is no paste buffer in the host. Yanked text goes wherever your plugin puts it.

`neotmux.home` is a fact, not an option: it sits beside `pane.info`'s facts rather than under
`opt`, because nothing sets it. The VM has no `os.getenv` — reading the process environment is
the host's alone — so this is the one value out of it a plugin ever sees, and it is the same
read the `~/.config/neotmux` layer is resolved from. It is `nil`, not `''`, where the process
had no `HOME`.

Logs go to `$XDG_STATE_HOME/neotmux/neotmux.log`, or wherever `--log` says — every
notification goes there too. A client that leaves a running session is told nothing on stderr:
the log has it, and a terminal handed back covered in what a plugin said an hour ago is noise.
The one exception is a session that ended before the client had a screen, which is said on
stderr because there was nowhere else to say it.

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
- **A float grant or a handle call answers `nil` plus a message.** That is the answer, not a
  break: `neotmux.float` recorded nothing, `move_to` and `resize_to` left the float where it is,
  and the tick keeps running — there is no raised-error path for a rect that is not a rect. Only
  `remove` is unanswerable, because it is already idempotent.
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

What an overrun looks like from inside your plugin: nothing you can catch. Your code stops
between two instructions with an error you did not raise, wherever it happened to be. A
coroutine does not help — the measurement follows into `coroutine.wrap` — and neither does
`pcall`: once the deadline has passed, `pcall`, `xpcall` and `coroutine.resume` re-raise the
budget's error instead of reporting it, so a loop wrapped in one still stops. An ordinary
failure in your own `pcall` is reported to you exactly as it always was; only the budget's own
error is unswallowable. In the log:

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

### The other places plugin code runs

Your key handlers and your event handlers are measured against the same 8 ms. An overrun there
is cut off and logged and that is all — it does not count towards the five, because what the
count buys back is a *tick* subscription and a key handler is not one.

Your `init.lua` is measured too, against `config_budget_ms` rather than the tick budget: a
load legitimately does far more work than a frame, and 8 ms would make plugins unloadable. Pass
it and the load fails the way any other load failure does, naming the file — neotmux comes up
without you rather than not coming up at all. The deadline is fixed when your file starts, so
setting `config_budget_ms` in one layer's `init.lua` changes the budget of the layers loaded
after it, never its own.

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

Nothing is measured and nothing is counted — no hook is armed at all, in a tick, in a key or
event handler, or during a load, whatever `config_budget_ms` says. This is what you want while
stepping through a plugin with a debugger, and it means a `while true do end` anywhere freezes
neotmux with no way out but killing it.

### What the budget cannot cut off

The hook that measures you fires between Lua instructions, so it cannot interrupt anything that
is not Lua instructions. `pcall` is no longer on this list — see above. It still checks the clock when your tick returns, so these are still
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
- **a host binding that blocks.** That would be our bug, not yours

None of this is a sandbox. Plugins are trusted (below); the budget is there so an *accident*
costs one plugin one frame instead of costing you the program.

## The shipped defaults

neotmux ships ten plugins in the runtime layer an install puts on the search path:

| plugin | what it does |
|---|---|
| `status` | the default status bar, on one reserved top row |
| `prefix` | a `C-b` prefix over `v`, `-`, `o`, `h/j/k/l`, `r`, `c`, `n`, `p`, `,` and `[` |
| `copy` | copy mode: read back over a pane's scrollback, select a region, yank it |
| `notify` | draws each notification in a frame of its own, stacked up from the bottom-right |
| `sessions` | the session picker on `C-b s`: every session and what it is running, `Enter` to go, `c` to make one, `r` to rename one, `d` to end one |
| `palette` | the command palette on `C-b :`: every registered command, fuzzy-filtered as you type, `Enter` to run the one under the cursor |
| `borders` | a rule along every boundary between two panes |
| `guard` | holds back the Enter that would run a line `neotmux.guard`'s rules name, until you press it again — **off by default** |
| `watch` | notifies you when a pane looks like it is waiting for you, on `neotmux.state`'s guess — **off by default** |
| `sidebar` | a tree of every session, and the windows and panes of the one you are in, down a reserved left band, folded by `C-b b` — **off by default** |
| `footer` | whichever mode is raised through `neotmux.mode`, on one reserved bottom row — **off by default** |

None of them is a host feature — all ten are ordinary files making ordinary API calls, and
the prefix is the only reason a fresh install can make a second pane at all. `C-b v` splits
vertically (the panes end up side by side, because `pane.split('vertical')` divides the
width), `C-b -` splits horizontally (one above the other), `C-b o` focuses the next pane,
`C-b h/j/k/l` focuses the pane to the left, below, above or to the right, `C-b r` enters
resize mode, `C-b [` enters copy mode, `C-b s` opens the session picker, `C-b :` opens the
command palette, `C-b c` opens a window and switches to it, and `C-b n` and `C-b p`
walk to the next window and the one before, each wrapping around.

Every one of the prefix's own verbs is a `neotmux.command`, registered by `plugin.prefix`
itself — the palette lists **commands, not keys**, so what it shows is whatever has been
registered, from any plugin, and a plugin that calls `neotmux.command.add` gets a palette entry
for free without doing anything else. `plugin.prefix`'s verbs are simply the first ones there,
which is what gives a fresh install something to open the palette onto.

Every mode moves with `j`/`k` (and `h`/`l` where sideways means anything): one spelling across
the picker, copy mode, pane focus and resize, so a hand that has learned one has learned all of
them. The arrows are deliberately not bound.
With the sidebar on, `C-b b` folds it — the sidebar adds that one itself, since it is the
optional plugin and the prefix is the standard one.

The sidebar is a tree: every session with what it is running, and under the one you are in, its
windows and their panes. Its width is taken from what there is to show, between
`require('plugin.sidebar').widths.min` (27) and `.max` (36). What is on which row is a table
rather than a shape baked into the file — each level draws one entry as a list of rows, and each
row is a list of tokens:

```lua
local sidebar = require('plugin.sidebar')
sidebar.sessions.rows = { { 'state_icon', 'name' }, { 'command', 'panes' } }
sidebar.panes.rows = { { 'marker', 'tree', 'number', 'command' } }
sidebar.tree.on = false          -- indent without the lines
sidebar.tokens.sessions.age = function(it) ... end   -- a token of your own
```

so moving what is where, or dropping what you do not care about, is an assignment rather than a
fork of the file. The first column is left to the markers on purpose: it is the whole of the
band when the sidebar is folded, and a fold showing a run of `│` says nothing about where you
are.

`h/j/k/l` are geometry, not list order: the pane you get is the one whose edge faces most of
yours on that side, which is what you would point at however deeply the splits are nested.
There is no wrapping — at the edge of the window the key does nothing, and `o` is still there
for walking every pane in turn.

Resize mode is a flag in the plugin and an `on_key`, like the prefix itself; the host has no
idea it exists. In it, `h/j/k/l` move the boundary one cell at a time
(`require('plugin.prefix').resize_step` sets how far), and the first key that is not one of
them leaves — `Esc` and `Enter` being the two that mean nothing else. Both the armed prefix and
resize mode raise their own name in `neotmux.mode` for as long as they are up — see
[The mode convention](#the-mode-convention) below — so `plugin.footer`, on, reads `prefix` or
`resize` rather than nothing.

### Turning one off

Every layer's `init.lua` runs, earliest layer first, and the shipped layer is last — so your
own `init.lua` can decide what the shipped ones do:

```lua
-- ~/.config/neotmux/init.lua
require('neotmux.defaults').status = false
require('neotmux.defaults').prefix = false
require('neotmux.defaults').borders = false
require('neotmux.defaults').copy = false
```

`watch`, `sidebar` and `footer` go the other way: they are off until you turn them on. What
counts as a pane waiting for you is an opinion about what you run, and twenty-four columns of
sidebar is an opinion about screen real estate; a status line is neither, and neither is copy
mode — it takes no band, no gutter and no cell from any pane, and what it reads is a buffer the
host is already keeping whether or not anything ever looks at it. `footer` is off for the same
reason `status` would be if it were optional: a row is a row whether or not a mode is ever
raised into it.

The sidebar has two widths rather than an on and an off — `require('plugin.sidebar').widths`
is `{ open = 24, closed = 1 }`, and `C-b b` swaps them. Closed is the markers column: you can
still see how many windows and panes there are and which one you are in, you just cannot read
their names. A band of nothing would be a band you have to remember is there.

```lua
require('neotmux.defaults').watch = true
require('neotmux.defaults').sidebar = true
require('neotmux.defaults').footer = true
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

An entry, keyed by the key spec that runs it — the same specs `keymap.set` takes — is either a
`neotmux.command` name, run through `neotmux.command.run` so the palette can run the same verb,
or a function of no arguments, for one that has not been registered. The prefix's own verbs are
all named (`split-vertical`, `focus-left`, `window-new`, and so on — see
[Commands](#commands)), so copying an entry, as `%` and `"` do above, copies a name, and one of
your own can be either. The prefix swallows the key after itself whether or not the table has
an entry for it, so a mistyped command never leaks a character into the shell.

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

### Changing what copy mode does

`plugin.copy` is `C-b [`, a cursor over a snapshot of the pane's history and screen, a
selection drawn with `ctx:tint`, and a yank. It reserves nothing and binds no key of its own:
the mode is a local in the file and an `on_key`, exactly as the prefix's own two modes are.

```
h j k l     the cursor, one cell
C-u C-d     half a screen
g G         the oldest line the snapshot holds, the newest
Space       start a selection here, or drop the one there is
y           yank it
Esc q       leave
```

**Keys and commands are two tables, not a chain of ifs.** `M.keys` maps a key spec to the
*name* of a command and `M.commands` maps that name to a function, and the key handler does
nothing but look one up in the other. Rebinding is then an assignment, and a command is
nameable from outside the file:

```lua
local copy = require('plugin.copy')
copy.keys['v'] = 'begin-selection'   -- vi muscle memory, alongside Space
copy.keys['C-b'] = nil               -- and one taken away
copy.colors.selection = '#45475a'    -- read every tick
copy.page = 0.5                      -- what C-u and C-d move, as a fraction of the pane
neotmux.log.info(copy.register)      -- what the last yank took
```

A key with no entry does nothing and is still swallowed. Resize mode leaves on the first key
that is not an adjustment because there is nothing in it to lose; dropping a half-made
selection over a typo is a different bargain, and `Esc` and `q` are both already the way out.

Note the one ordering trap: `require`ing this file from your own `init.lua` — which is what
rebinding a key means — loads it before the shipped layer runs, so it loads the prefix itself
rather than probing for it. Otherwise `C-b [` would silently go unbound.

#### The buffer is taken once

Entering the mode reads the pane's whole history and its screen, in one pass, and never reads
them again. That is not a cache. The child keeps printing while you are reading it, and a
buffer re-read every frame is a buffer whose line 400 is one thing when the cursor lands on it
and another when it is yanked — so what you copy would not be what you selected. tmux clones
the grid on the way in for exactly this reason and reads only the clone.

It costs what it says: a full ten-thousand-line history is about **11 ms** to snapshot, on one
key press, outside any tick and therefore outside the budget. tmux's version of this is a
memcpy; ours builds ten thousand Lua strings out of the terminal's cells, which is three
orders of magnitude worse and is the strongest argument for a host call that freezes a pane
instead.

Two consequences you can see. A pane that prints while you are in the mode does not change what
the mode shows — it shows the snapshot. And because the pane's own grid has then moved out from
under the snapshot, the frame stops being free (below).

#### What the view costs per frame

The pane's own grid is what the host renders. There is exactly one case where that grid is
already showing what the mode wants shown — the view sitting on the snapshot's own screen over
a pane that has not been fed since — and then the frame is **one `ctx:tint` for the cursor and
nothing else**. Scrolled, or overtaken by output, every row is in the wrong place: the pane is
erased and redrawn out of the snapshot, one `ctx:rect` and one `ctx:text` per non-blank row,
which on a 23-row pane is **25 commands**. Measured on a release build against a
ten-thousand-line buffer: 8 µs a frame sitting still, 14 µs scrolled, 32 µs with the cursor
moving every frame, against a budget of 8 ms.

No read is on that bill. The lines were taken on the way in; the tick slices a table.

A line out of the snapshot has no colours on it — the ABI hands back text — so a redrawn view
is monochrome, and the selection's tint is the only colour in it. Sitting on the pane's own
screen the colours are all there, because nothing is drawn over them.

#### The selection is two absolute points

`{ line, col }` twice, where `line` is a line number in the buffer and never a screen row. A
selection held in screen coordinates evaporates the moment the view scrolls under it; held
against the buffer it survives, and `neotmux.selection.span` projects it back onto whichever
rows are on screen — an end that has scrolled out of view is clamped to the edge rather than
lost, and a view that has left the selection behind draws nothing rather than forgetting it.

#### Where a yank goes

Into `require('plugin.copy').register`, and into the system clipboard if this host can reach
one — which today it cannot, so the notification says the register and does not claim
otherwise.

Neither of the two routes that would stay inside Lua is open. **OSC 52** needs a plugin to
write bytes to the outer terminal, and there is no such call: `io` is nil in the VM, and every
drawing call puts a character into a cell of the surface's grid, where an escape byte is a
glyph and not an escape. **Writing the text into a pane** needs a way to send input to one, and
the pane API splits, focuses and resizes — that is all of it. So the internal buffer is the
whole of it, and it is deliberately separate from the clipboard so that a paste command can
attach to it later without caring whether a clipboard existed. A clipboard call, when there is
one, must say whether the terminal advertised the capability: reporting a copy that never left
the process is worse than reporting none.

#### Two things it gets wrong, on purpose rather than by accident

**A soft-wrapped line is yanked with a newline in the middle of it.** A terminal wraps a long
command across two rows, and the right answer is to join them with nothing at all — the rule
tmux keeps in `window_copy_copy_line`. It can keep it because its grid says which rows are
continuations. This ABI hands back lines and nothing else: there is no wrap flag, and none can
be inferred, because a wrapped row and a row that happens to fill the width read identically
once trailing blanks are cut. Guessing would put the defect somewhere unpredictable instead of
somewhere stated. It wants a `wrapped` flag on the lines `content` and `scrollback` return.

**A slice of a line holding a double-width character can be a character short.** The host hands
a wide character back once, at the column it starts in, so a line's character indices and its
columns part company. Everything drawn is in columns, because `ctx:tint` counts columns;
`selection.slice` counts characters, because that is all a Lua string offers. An ASCII line is
exact; a CJK one drifts left by one per wide character before the cut. It wants a way to
measure a string in columns.

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

### Showing a pane's own history

```lua
ctx:view(pane, back)
```

The pane is drawn `back` lines into its own scrollback for this frame, and for this frame only:
nothing about the session moves, the child is not told, and the next frame that does not ask
shows the pane as it is. What you get is the pane's **own cells** — every colour, every
attribute, exactly what the child wrote — which is the thing you cannot have by reading
`pane.content` and drawing it back, because a line out of this ABI is text.

That is what copy mode is built on. Pair it with `pane.freeze`, or the view slides under you
the moment the child prints:

```lua
neotmux.pane.freeze(pane)          -- on the way in
ctx:view(pane, back)               -- every tick, from your own idea of where you are
neotmux.pane.unfreeze(pane)        -- on the way out
```

Two sources asking for the same pane is the later one winning, the rule the overlay already
follows for the same cells. A view is committed and discarded with your drawing, so a tick that
overran leaves no pane scrolled.

### The notification surface

`neotmux.notify` raises one and the host holds it: it expires them on its own clock and keeps
the transcript a client attaching is told. What one *looks like* is a plugin's,
the same way a status bar is — `plugin.notify` is the shipped one, and
`neotmux.notifications.list()` is what it reads.

The surface is the host's on purpose. A source's own draw buffer is discarded when that source
overruns, so a notification about the Lua layer failing cannot live in it; raised here, it
outlives whatever raised it. What a plugin drawing them cannot survive is its own failure —
and that one is in the log, which is where a plugin that has stopped drawing is diagnosed
anyway.

### The prompt convention

One line of text, asked for in a float. The host has none of this — a float, a mode, an
`on_key` over a local and a caret drawn as a rect are the whole of it — so `neotmux.prompt` is
where the shipped one lives, beside `neotmux.mode` and `neotmux.box`.

```lua
require('neotmux.prompt').open({
  title = 'name window',
  value = neotmux.window.name(id) or '',
  done = function(text) neotmux.window.rename(id, text ~= '' and text or nil) end,
})
```

`done` is called with what was typed when Enter is pressed, and not at all when Esc is. The
caret is drawn rather than placed: the terminal's own cursor belongs to the focused pane, and a
float cannot take it — which is what a text field does everywhere else too.

`C-b ,` is the shipped use of it, naming the window you are in.

### The list convention

A keyboard-driven list in a float — a cursor that moves and wraps, a window that follows it,
and, where the caller wants one, an input line for a filter or for asking a question mid-list.
The session picker and the palette below grew their own cursor, their own scrolling and their
own row drawing before this existed; `neotmux.list` is those parts with the data taken out, in
the same spirit `neotmux.prompt` and `neotmux.box` are one line and one frame with the data
taken out.

```lua
local list = require('neotmux.list')
local handle = list.open({
  title = 'sessions',
  items = function() return neotmux.session.list() end,
  row = function(item, chosen) return { { text = item } } end,
  done = function(item, handle) neotmux.session.switch(item) handle:close() end,
})
```

`items` is read fresh on every tick, so a list stays current while it is up. `row` answers a
list of runs — `{ text =, fg =, bg = }` — drawn left to right and clipped to the row's width,
the same shape `ctx:text` takes one of. `done` fires on `Enter` and is not itself a close: the
handle is the caller's to close, so a rename or a kill can run one and leave the list open,
which is why `plugin.sessions`'s own `done` calls `handle:close()` and its `r` and `d` do not.
`cancel` fires on `Esc`/`q` instead, when given. `keys` adds more of them, keyed by spec and
called with the item under the cursor and the handle, the same as `done`.

Give `filter` a predicate — `function(item, query) return ... end` — and the list becomes one
you type into rather than move through by letter: the input line is always up, every printable
key joins the query, and the cursor moves on `C-j`/`C-k` instead of the plain `j`/`k` a
filter-less list uses, because a filtered list still needs `j` and `k` to be characters a query
can hold. `handle:query()` reads what has been typed, and `items` reading it back is how a list
ranks or narrows itself rather than merely keeping or dropping in place — see `plugin.palette`,
below, whose own `items` sorts by a fuzzy score computed against `handle:query()` and whose
`filter` only confirms what `items` already decided.

`handle:ask({ label =, value =, accept =, done =, cancel = })` raises the same input line for a
question that is not the filter — a name to create or rename, say — with `accept(key, text)`
given the chance to refuse a character before it joins the buffer. One list is open at a time:
opening a second closes the first, the same rule `neotmux.prompt` follows.

### The state convention

The host knows a pane's bytes and a stamp that moves when they change. Whether that means the
thing in it is *working*, *blocked* on a question, or *idle* is a guess about text on a screen,
and `neotmux.state` is where the shipped guess lives — so the plugin that notifies you and the
sidebar that draws you a list agree about what a pane is doing.

```lua
local state = require('neotmux.state')

state.of(pane)          --> 'working' | 'blocked' | 'idle', or nil for a pane that went away
state.worst(panes)      --> the same three, for a set: blocked beats working beats idle
state.icon('blocked')   --> ◉

state.patterns          -- the Lua patterns that mean "this is a question"; replace the table
state.lines = 5         -- how many of a pane's last non-blank lines to match against
state.working_for = 2   -- seconds after a pane last said something that it still reads as working
```

The expensive half — reading the pane's text — happens only for a pane whose revision has moved
since it was last asked about, so a session nobody is typing into costs one stamp per pane per
tick and nothing else. `of` is answered on demand rather than on a tick of its own: every caller
is already in one.

The three words are herdr's, and so is what they are for: an agent you are running is either
getting on with it, waiting for you, or doing nothing, and which of those it is, is the thing
you open a sidebar to find out.

### The box convention

The host draws nothing into a float: it hands you a rect and a coordinate space, and the cells
are yours. So a window that reads as a window — a line round it, its content held off the edge —
is a convention, and `neotmux.box` is where the shipped one lives, in the same spirit as
`neotmux.mode`.

```lua
local box = require('neotmux.box')

neotmux.on_tick(function(ctx)
  local rect = ctx:float_rect(win)
  if not rect then return end
  local inner = box.draw(ctx, win, rect, { title = 'sessions' })
  ctx:text(win, inner.x, inner.y, 'the first row of your content')
end)
```

`draw` fills the rect, puts a frame with rounded corners on its edge, and hands back the rect
*inside* the frame, so the arithmetic is done once. `M.glyphs` and `M.colors` are read on every
call, so an `init.lua` that wants square corners or another palette assigns to them. A float
too small to hold a frame is filled and handed back whole rather than drawn over.

### The mode convention

The host has no concept of a mode. What makes a plugin modal is entirely on the plugin's side —
returning `true` from `on_key` is the whole of it — so nothing outside that plugin's own file
could previously tell a mode was up at all.

`neotmux.mode` is a Lua module, not a host feature, that gives modal plugins somewhere to say
so:

```lua
local modes = require('neotmux.mode')

local handle = modes.raise('my-mode')   -- entering
-- ...
handle:lower()                          -- leaving

modes.current()                         -- the most recently raised name, or nil
```

Raising returns a handle, the same shape `neotmux.reserve` and every `on_*` registration
already return, and for the same reason: `handle:lower()` takes back exactly the entry it was
handed, never anyone else's, even one raised under the same name. Raised names stack, so if a
second mode is raised while a first is still up, `current()` reports the second one, and
lowering the second uncovers the first again — lowering never reaches past your own entry.

A reader never learns who raised what it sees. That is what lets `plugin.footer` show a
third-party plugin's mode with no knowledge of that plugin at all — it only ever calls
`modes.current()`.

`plugin.prefix` raises `'prefix'` while armed and `'resize'` in resize mode, and lowers each on
the way out; `plugin.copy` raises `'copy'` on entry and lowers it on every exit — the explicit
`q`/`Esc`, the pane closing, and focus moving to another pane.

Nothing catches a plugin that raises and forgets to lower. There is no host-side hook for it,
because there is no host-side idea of a mode to hang one on — the same bargain a leaked
`neotmux.reserve` band already makes. A plugin that never lowers leaves its name on top for
good, and every reader, `plugin.footer` included, keeps showing it. That is a bug in the
plugin, found the same way a leaked band is: by looking at the screen.

### Changing what the footer shows

`plugin.footer` reserves the bottom row and draws whichever name `neotmux.mode.current()`
answers, or nothing at all when that is nil:

```lua
local footer = require('plugin.footer')
footer.colors.bar = '#181825'
footer.colors.text = '#f9e2af'
```

Both are Catppuccin Mocha, matching `plugin.status`, and both are read every tick. There are no
keys — the footer only reads the convention above, it does not raise a mode of its own.

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

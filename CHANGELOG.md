# Changelog

## Unreleased

### A band can be taken off the left or the right

`neotmux.reserve` now takes `'left'` and `'right'` as well, measured in **columns** rather
than rows. Everything else about a reservation is unchanged: a handle back, one reservation
per source, `ctx:reserved()` answering that source's own band in surface coordinates, and
bands allocated in load order from the outside in.

`left` and `right` were deliberately absent until now, on the grounds that the consumers were
a status line and a window list and nobody wanted a vertical band. A sidebar is that consumer,
so the omission stopped being right.

The two axes are bounded separately: four rows on a horizontal edge as before, forty columns
on a vertical one. Four is where a line or two of text stops being a line of text; forty is
half of the eighty columns a terminal is still written against, past which a band is not a
margin on the panes but the layout.

They are also *refused* separately. A terminal that cannot spare the rows and still leave the
panes a grid `vt100` survives applies none of the horizontal bands — and keeps the
vertical ones, because rows and columns are spent out of different room and hiding a sidebar
over a shortage of rows would give the panes nothing back for it.

A horizontal band spans the full width and a vertical one only what the horizontal bands
left, so no two bands ever share a cell.

### A reserved band can change size

The handle `neotmux.reserve` answers now has a second method: `handle:resize_to(cells)`, the
same band on the same edge at a different size, from the next frame.

It is a resize rather than a remove and a fresh reservation because bands are laid out in the
order they were reserved, outward in: a plugin that gave its band back and asked for another
would come back inside everyone who reserved after it, so anything that folded would walk
across the screen. Resizing edits the reservation in place and keeps its place on the edge.

The refusals are the ones `reserve` already draws. Zero and a count past the edge's own bound
raise, because they are typos — there is no band of no cells, so `ctx:reserved()` never
answers a rect a plugin must not draw in. Growing into cells another plugin holds answers
`nil` plus a message. The cap is counted against the other bands on the edge only, so a band
never pays for its own cells twice.

`neotmux.pane.resize` moves a boundary by a delta, which is why this one is spelled
`resize_to`.

### A sidebar, shipped and off by default

`plugin/sidebar.lua` lists every window, every window's panes and what each pane is running,
down a reserved left band. It marks the window being shown and the focused pane. It is off
until you turn it on — `require('neotmux.defaults').sidebar = true` — like the
watcher and unlike the status bar: a status line spends a row, a sidebar spends a third of the
width.

It is an ordinary file making ordinary API calls, and it needed no new ones. A pane in a
window nobody is looking at shows an em dash instead of a command, because `pane.info` answers
about the shown window's panes and no others.

`C-b b` folds it, when the prefix is loaded too: the sidebar puts that command in the prefix's
own table, because the sidebar is the optional plugin and the prefix is the standard one.
Folded is a one-column rail rather than nothing — the markers column, so the shape of the
session is still there to see and only the names are gone. The rail is the open sidebar with
everything that does not fit cut off, so there is no second way of drawing it.

### A plugin can ask which panes a window has

`neotmux.window.panes(id)` answers with a window's panes in layout order, for any window the
host owns, hidden or not. Until now nothing mapped a window to its panes: `pane.list()` is the
shown window's list and `window.list()` is only ids, so a plugin discovered a hidden pane by
hearing it speak. A pane that had been silent since it opened therefore did not exist as far
as any plugin was concerned — and a window opened with `window.create()` starts exactly that
way, since it raises `window_opened` and no `pane_opened`.

The order is the one `pane.list()` gives: left to right, top to bottom. A hidden window keeps
the rects it was last assigned, so it has a layout too and gets the same answer — the
arrangement it will be shown in. An id no window has answers `nil`, the way `pane.current()`
and `pane.rect()` answer nothing; a living window always holds at least one pane, so an empty
list is not something you can be told.

`pane.list()` is unchanged: it is still the panes of the session in front of the user. And
finding is all this adds — there is still no way to split, focus or resize a pane in a window
nobody is looking at.

The shipped watcher uses it. It enumerates every window's panes on each tick instead of
learning ids from `pane_output`, so an agent started in a window you are not looking at is
watched from the moment it exists rather than from the moment it first prints something. The
revision gate is untouched: a pane's lines are still built only when its terminal has been fed
since it was last read, so an idle session still reads nothing.

### The dividers meet where they cross

Four panes in a grid used to leave a hole at the crossing. Each pane draws the divider down
its own right-hand gutter and across its own bottom one, so the cell where the two gutters
meet is inside neither pane's extent and nobody drew it.

It is filled now, with the glyph the arms deserve rather than a `┼` everywhere: a cell has an
arm in a direction when a divider covers the neighbouring cell that way, so a full-height pane
beside a split column gets `├` and a wide pane above two gets `┬`, and no arm is ever drawn
into a pane that is not divided there. Working that out is `neotmux.junctions`, a pure
geometry module beside `neotmux.nav`; `plugin.borders` decides what to put in the cell, and
its five new fields — `cross`, `tee_up`, `tee_down`, `tee_left`, `tee_right` — are read every
tick like `vertical` and `horizontal`. The host learned nothing: a junction is one more cell
of `ctx:line`.

### The bar counts panes and windows the way a person does

Split, end the shell in the new pane, split again: the number on the bar used to climb — 2,
then 3, then 4 — with never more than two panes on screen. The ids were right. A `PaneId` is
allocated once and never reused, and a `WindowId` likewise, because a plugin holding id 2 must
not find that it now means something else. Showing one to a person was the mistake.

The bar shows positions instead: where the pane sits in `pane.list()`, which is in layout
order, and where the window sits in `window.list()`. Both close the gap when one goes, so a
block is numbered the way tmux numbers them. Nothing in the host changed and no id was
renumbered — `status/position.lua` is a loop over a list the bar already had.

A block now reads `1.2  nvim`: the window's position, the focused pane's position, the command.

### Panes you can move between and resize

`neotmux.pane.resize('left' | 'right' | 'up' | 'down', cells)` moves the boundary between the
focused pane and its neighbour that way, by exactly the cells asked for. One rule and no
exception: tmux grows the pane unless it is at the edge, where it silently moves the other
boundary instead, and nobody can predict that. With no neighbour that way there is no
boundary and the call is refused. The boundary moved is the nearest one above the pane, so a
pane three splits deep resizes against what it is actually touching.

Cells, not a fraction: the tree still stores an `f32`, but a resize reads its current
boundary back through the same rounding `assign` lays it down with, so ten moves of one cell
move ten cells rather than nine.

`neotmux.pane.rect(id)` answers the rect `ctx:pane_rect` answers, out of the same published
geometry, from anywhere — geometry was readable only inside a tick, so a plugin that wanted
to know which pane was to its left had to register a draw callback it did not want. `list()`
was already readable anywhere and threw the rects away; that asymmetry is gone.

The shipped prefix uses both: `C-b h/j/k/l` moves the focus to the pane whose edge faces most
of yours on that side, and `C-b r` enters a resize mode where the arrows and `h/j/k/l` move
the boundary. Both are ordinary Lua in `plugin/prefix.lua`, in the same command table the
rest of the prefix is in, and the mode is a file-local flag — the host learns nothing about
modes.

### Keystrokes that no longer wait for the frame

A keystroke used to cost a whole frame. The loop waited on one of the two things that make a
frame worth drawing — the terminal's own input — and collected each pane's output with a
non-blocking read once per pass, so a shell that echoed a character a millisecond later went
unnoticed until the next frame boundary. Measured keystroke to first byte back, typing into
bash: a median of 20 ms against a bare PTY's 0.2 ms.

Every pane's reader thread now nudges a pipe the wait watches alongside the terminal, and the
frame's work starts as soon as there is a reason for it rather than at the next deadline. The
same measurement is a median of 1.3 ms.

A frame is still 16 ms and still draws at most once. What changed is where the drawing sits
inside it: the frame waits for its reason, draws, and spends the rest of its time pumping
input rather than asleep, so a key still reaches its pane the moment it arrives and a pane's
output waits for the next frame rather than drawing this one twice.

### What a pane is doing

`neotmux.pane.info(id)` answers with the pane's foreground command and that process's working
directory — `{ command = 'nvim', cwd = '/home/you/src' }` — which is the content a status line
was missing. It is the pane's foreground process group rather than the shell neotmux spawned,
so a pane running `nvim` says `nvim` and the same pane back at its prompt says `bash`.

An unknown id answers `nil`, and a fact that could not be read is an absent field; neither
raises, because a shell exiting is not a plugin's error. The facts are sampled by the host at
most every 250 ms and a call reads the last sample, so asking on every tick for every pane
costs no syscalls. Read from `/proc`: Linux-only, and unknown everywhere else.

### A status bar, not a debug line

The shipped `status` plugin now reserves the **top** row and draws what a status bar is for:
every window on the left, the one being shown capped in colour with what its focused pane is
running beside it, and on the right that command and its working directory, each behind its
own icon. `panes 2 focus 1` is gone.

It is composed from scratch every tick, so it follows the focus, a command starting or
finishing and a `cd` with no keystroke to prompt it. The directory is written the way a shell
prompt writes it — `$HOME` as `~` — and loses its leading components when the space runs out.

`$HOME` reaches it as `neotmux.home`, the one value out of the process environment the host
publishes: the VM has no `os.getenv`, and this is the same read the `~/.config/neotmux` layer
is resolved from, so the two cannot disagree. It is `nil` where the process had no `HOME`, and
the path is then written out in full.

The colours (Catppuccin Mocha), the caps and the icons are fields on the module, read on the
tick: `require('plugin.status').caps = { left = '', right = '' }` is the whole escape for a
terminal without a Nerd Font.

### The status line is a plugin now

`neotmux.reserve('bottom', 1)` takes a band of the surface and returns a handle; the panes get
what is left, and the plugin draws into the rect `ctx:reserved()` hands it with the same
`ctx:text` it would use anywhere else. `ctx:status` and the status row that came with it are
gone — the host no longer knows what a status line is, only how to hand out rows along an
edge. The shipped `status` plugin looks the same on screen and is 20 lines of ordinary API
calls.

`top` works as well as `bottom`, two sources on one edge get a row each — outermost to
whoever asked first — and a band is outside every pane's rect, so nothing a shell writes can
reach one. An edge holds at most four rows; a terminal too small for the bands it granted
hides them for the frame and gives them back when it grows, the same way it hides a pane too
small to show.

### Panes you can see

`borders`, a third shipped plugin, rules every gap between two panes with `│` and `─`.

The host reserves the gap and the plugin draws in it, the same bargain as a reserved band:
`neotmux.opt.pane_gutter` says how many cells to leave between panes, and the host never
learns what anyone puts there. Without it a divider had nowhere to go but on top of a cell a
shell was using — a vertical split of an 80-column terminal silently ate the 40th character
of every line in the left pane. Still no adjacency API: with a gutter of `g` a pane's right
neighbour starts at `r.x + r.w + g`, so `pane_rect` and `line` remain the whole story.

`ctx:line` now takes an optional glyph, so the character is the plugin's choice rather than
the host's. Omit it and you get the `#` it always drew.

### A prefix you can change

The shipped prefix is now `C-b`, with `v` splitting vertically, `-` horizontally and `o`
walking to the next pane. Its command table is a field of the module rather than a file-local,
so a user's `init.lua` can add, replace or drop a binding — `prefix.commands['%']` back to
tmux's is one line — instead of only being allowed to move the prefix key.

## v0.1.0 — 2026-09-20

The line at which a plugin author can exercise the ABI. **Not** the line at which neotmux is
a daily driver: there is no detach/attach, and that is the intended trade — the ABI is the
part that is expensive to change after publication, so it meets other people before it
freezes, and the server/client split happens against an ABI that has been used rather than
one that has not.

### What it is

A terminal multiplexer whose core is pane management and whose everything else is a Lua
plugin. tmux is fast but has no in-process extension ABI, so "install a plugin" can only mean
"run something that mutates your config". zellij has a real plugin system but pays for it
with a sandboxed WASM interpreter on the render path. nvim's answer is to trust the plugin,
run it in process, and give it a real ABI — and neotmux adds the piece nvim lacks: **a plugin
tick is a best-effort operation that is allowed to fail.**

### The host has no plugin concept

It provides an API; a loading convention decides what gets loaded; a plugin is a bundle of
API calls and is a plugin only as a result. Making that true is most of what this release is.
Identity is the chunk a piece of code was written in, and attribution is lexical everywhere —
where the calling code was *written*, never where it runs from — so a misbehaving plugin can
be named whether it fails during load, a tick, a key, an event, or inside a coroutine.

Five things had to go for it to be true: the plugin identity itself, a warning that loaded
code ought to register something, a rule that isolated only modules named `plugin.*`, a
reserved prefix key, and three events plus a status hook the binary could never fire.

Removing the prefix reservation is what proved the ABI was short a piece: a prefix could only
intercept keys it had already bound, so it was not modal and it starved every other plugin of
those keys. `neotmux.on_key` closed that, and the prefix now ships as a plugin that binds
nothing.

### A tick is allowed to fail

A global Lua debug hook samples every 1000 instructions. A tick that passes its budget is
unwound; that source's frame and the ops it pushed are discarded, the overrun is logged
against its name, and five in a row park its tick registrations — handed back by
`neotmux.resubscribe(name)`, because the alternative to recovery is restarting neotmux, which
kills every shell you are running.

What the hook cannot cut off it still attributes: a tick that spends 189 ms inside one
`string.rep` is named and counted on the way out, and parked if it keeps doing it. Being
unable to interrupt is acceptable; being unable to attribute is not.

### Panes

A binary split tree, one PTY and one grid per pane, and splits, focus, resize and close
driven entirely from Lua. The host binds no key and has no prefix: `C-a %` is a plugin.

### The ABI

`on_tick`, `on_event`, `on_key`, `keymap.set`, `neotmux.pane.{list,current,split,focus}`,
immediate-mode drawing through `ctx`, `opt`, `log`, `notify`, `quit`, `resubscribe`.
Documented in `docs/plugins.md`, with three working plugins in `examples/`.

### Not in this release

detach/attach (v0.2), scrollback and copy-mode, mouse, multiple sessions, a plugin manager,
`pane.close` and `pane.resize`, and tmux compatibility in any form.

### Install

```sh
make install            # cargo install plus the shipped runtime layer
```

`cargo install` alone copies only the binary; the default status line and prefix live in a
runtime layer that needs its own step. A C compiler is required — `mlua` builds Lua 5.4 from
vendored source. Nothing else is.

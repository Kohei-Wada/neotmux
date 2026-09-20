# Changelog

## Unreleased

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

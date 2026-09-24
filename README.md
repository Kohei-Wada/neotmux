# neotmux

A terminal multiplexer whose core is pane management and whose everything else is a Lua plugin.

Not a tmux fork and not tmux-compatible: no `.tmux.conf`, no tpm, no `tmux` CLI surface.
The point is the extension contract tmux never had.

## Why

tmux is fast and general but has no in-process extension ABI — plugins are executables that
call the `tmux` CLI from outside, so "install a plugin" can only ever mean "run something that
mutates your config". zellij has a real plugin system but pays for it with a sandboxed WASM
interpreter on the render path. nvim shows the third option: trust the plugin, run it in
process, and give it a real ABI.

neotmux takes nvim's answer and adds the piece nvim doesn't have — **a plugin tick is a
best-effort operation that is allowed to fail**. Plugins get an instruction budget; a plugin
that blows it is named, dropped for that frame, and unsubscribed if it keeps doing it. The
failure stays partial and attributable instead of freezing the whole UI.

And it puts that ABI in the multiplexer rather than the editor, because the multiplexer is
the layer every program's input and output passes through — an editor should stay an editor,
and a plugin here reaches the shell, the agent and nvim itself alike.

## Status

Pre-release, and the plugin ABI is not stable yet.

Working today: a shell running full-screen on a PTY, and a Lua layer that loads code from
`$XDG_CONFIG_HOME/neotmux`, lets it register tick, event and keymap callbacks, reserve a band
of the surface to draw in — rows at the top or bottom, columns at the left or right — and float
a window over the panes, draw into the frame, and log, with every registration and every API
call attributed to the chunk it was written in, so a misbehaving one can be named.

## Scope

**v0.1** is the line at which you can write a plugin against the ABI. It is not the line at
which neotmux is a daily driver. Getting there needs the per-frame instruction budget and the
pane tree — splits, focus, resize, prefix key — plus the ability to float a window over
the panes (`docs/plugins.md` has a section for it, next to the band's), and a default
status line shipped as a plugin rather than built in.

Detach and attach are in. `C-b d` (or `neotmux.detach()`) leaves the session running and
exits the client; running `neotmux` again finds it and attaches instead of starting a new
one. Nothing is lost to a detach: the panes, the shells, the layout, each pane's scrollback
*and the plugins* are the server's. A plugin keeps ticking while no terminal is attached,
which is what makes it possible to write one that watches a long-running job, and a copy
register or a raised mode is still there when you come back.

More than one session is in, and they live in one server behind one socket. `neotmux work`
starts that session or attaches to it, `ls` lists them, `kill [name]` ends one and the server
goes when its last session does. A name is a word a person types and reads rather than a
filename — one to sixty-four characters, none of them a control character, and not `-` first,
so `作業` is a session name. The shipped picker on `C-b s` lists every session with what it is
running and makes, switches to or ends one.

Two terminals can be in one session at once: a key typed on either reaches the same pane, and
both see the same plugin state, because there is one Lua layer for the server and the modes,
floats and bands it holds belong to the session rather than to whoever is looking at it.

Copy mode is in, as a plugin: `C-b [` reads back over a pane's scrollback with the vim
motions — `h/j/k/l`, `w/b/e`, `0/^/$`, `C-u/C-d`, `g/G` — `v` starts a selection and `y` yanks
it. The view is the pane's own cells shown at a place in its own history (`ctx:view`), so it
keeps every colour the child wrote. What is still out is the system clipboard, which this ABI
cannot reach, so a yank lands in a Lua register and is offered to the terminal over OSC 52.

Also out of v0.1: mouse, a plugin manager, and tmux compatibility
in any form.

Writing one: `docs/plugins.md`, with three working plugins in `examples/`.

See `docs/superpowers/specs/` for the design and `docs/superpowers/plans/` for the
implementation plans.

## Install

```sh
make install            # or: make install PREFIX=/usr/local
```

`mlua` builds Lua 5.4 from vendored source, so a C compiler has to be on the machine. Nothing
else is needed — no system Lua, no shared libraries.

`cargo install` on its own is not enough. It copies the binary and nothing else, and the
default status line is a plugin: it lives in the shipped runtime layer under `runtime/`, which
has to be installed alongside the binary as `<prefix>/share/neotmux`. That is what `make
install` adds, and it is the directory neotmux looks for next to its own executable. A binary
installed with `cargo install` alone starts and runs a shell with no status line and no error;
pointing `NEOTMUX_RUNTIME` at a checkout's `runtime/` is the other way to give it one.

Deleting `<prefix>/share/neotmux/lua/plugin` turns the shipped defaults off. Nothing in the
program knows what they said.

The shipped status bar takes the top row: the windows on the left, and the focused pane's
command and working directory on the right. It draws with Nerd Font glyphs — a rounded cap
either side and one in front of each module. Without such a font, two lines in your own
`init.lua` take them off and leave the colours:

```lua
require('plugin.status').caps = { left = '', right = '' }
require('plugin.status').icons = { command = '', directory = '' }
```

`docs/plugins.md` has the colours, and the `home` field the `~` comes from.

Configuration lives in `$XDG_CONFIG_HOME/neotmux` (or `~/.config/neotmux`): `init.lua` at the
top, and modules under `lua/`, so `require('plugin.clock')` loads `lua/plugin/clock.lua`. The
directory layout is a convention of where `init.lua` looks, not something the program enforces.

```sh
make run     # run it from a checkout
make check   # fmt + clippy + tests
```

## License

Dual-licensed under MIT or Apache-2.0, at your option — with one exception: the two sounds in
`runtime/sounds/`, `runtime/lua/neotmux/detect.lua` and `runtime/lua/neotmux/agents/claude.lua`
are derived from [herdr](https://github.com/herdrdev/herdr) and are available under Apache-2.0
only, since Apache-2.0 material cannot be taken under MIT. `NOTICE` says which files, from
where, and at what version.

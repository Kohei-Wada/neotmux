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

## Status

Pre-release, and the plugin ABI is not stable yet.

Working today: a shell running full-screen on a PTY, and a Lua layer that loads code from
`$XDG_CONFIG_HOME/neotmux`, lets it register tick, event and keymap callbacks, reserve a band
of the surface to draw in — rows at the top or bottom, columns at the left or right —
draw into the frame, and log, with every registration and every API call attributed to the
chunk it was written in, so a misbehaving one can be named.

## Scope

**v0.1** is the line at which you can write a plugin against the ABI. It is not the line at
which neotmux is a daily driver. Getting there needs the per-frame instruction budget and the
pane tree — splits, focus, resize, prefix key — plus `docs/plugins.md` and a default status
line shipped as a plugin rather than built in.

v0.1 deliberately does **not** include detach/attach. That is v0.2, and it is the largest
structural change on the roadmap (the server/client split), which is exactly why it should
happen against an ABI other people have already used instead of one nobody has touched.

Also out of v0.1: scrolling a pane back and copy-mode — a plugin can read the
scrollback, but nothing moves the view — plus mouse, multiple sessions, a plugin
manager, and tmux compatibility in any form.

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

Dual-licensed under MIT or Apache-2.0, at your option.

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
`$XDG_CONFIG_HOME/neotmux`, lets it register tick, event, keymap and status callbacks, draw
into the frame, and log — with every registration and every API call attributed to the chunk
it was written in, so a misbehaving one can be named.

## Scope

**v0.1** is the line at which you can write a plugin against the ABI. It is not the line at
which neotmux is a daily driver. Getting there needs the per-frame instruction budget and the
pane tree — splits, focus, resize, prefix key — plus `docs/plugins.md` and a default status
line shipped as a plugin rather than built in.

v0.1 deliberately does **not** include detach/attach. That is v0.2, and it is the largest
structural change on the roadmap (the server/client split), which is exactly why it should
happen against an ABI other people have already used instead of one nobody has touched.

Also out of v0.1: scrollback and copy-mode, mouse, multiple sessions, a plugin manager, and
tmux compatibility in any form.

See `docs/superpowers/specs/` for the design and `docs/superpowers/plans/` for the
implementation plans.

## Install

```sh
cargo install --git https://github.com/Kohei-Wada/neotmux
```

`mlua` builds Lua 5.4 from vendored source, so a C compiler has to be on the machine. Nothing
else is needed — no system Lua, no shared libraries.

Configuration lives in `$XDG_CONFIG_HOME/neotmux` (or `~/.config/neotmux`): `init.lua` at the
top, and modules under `lua/`, so `require('plugin.clock')` loads `lua/plugin/clock.lua`. The
directory layout is a convention of where `init.lua` looks, not something the program enforces.

```sh
make run     # run it from a checkout
make check   # fmt + clippy + tests
```

## License

Dual-licensed under MIT or Apache-2.0, at your option.

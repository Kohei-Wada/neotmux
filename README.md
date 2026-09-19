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

M0a (walking skeleton) works: `neotmux` runs a shell full-screen and exits cleanly.
There is no plugin system yet — that is M1a. See `docs/superpowers/specs/` for the design
and `docs/superpowers/plans/` for the implementation plans.

```sh
make run     # run it
make check   # fmt + clippy + tests
```

## License

Dual-licensed under MIT or Apache-2.0, at your option.

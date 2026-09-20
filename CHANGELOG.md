# Changelog

## Unreleased

### Panes you can see

`borders`, a third shipped plugin, rules every boundary between two panes. The panes tile the
surface exactly, so a pane edge that is not an edge of the surface is shared with a neighbour
and the divider goes on the boundary itself — no host change, no adjacency API, `pane_rect`
and `line` and nothing else.

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

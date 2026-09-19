# neotmux — M0 + M1 design

Date: 2026-09-19
Status: proposed
Scope: M0 (walking skeleton) and M1 (Lua plugin ABI). Later milestones are sketched
under "Roadmap" for context only and are not specified here.

## 1. Goal and non-goals

### Goal

A terminal multiplexer whose core is pane management and whose everything else is a Lua
plugin, published as OSS.

The thesis: tmux is fast and general but has no in-process extension ABI, so "install a
plugin" can only mean "run something that mutates your config". zellij has a real plugin
system but pays for it with a sandboxed WASM interpreter on the render path. nvim shows the
third answer — trust the plugin, run it in process, give it a real ABI.

neotmux takes nvim's answer and adds the piece nvim lacks: **a plugin tick is a best-effort
operation that is allowed to fail.** A plugin gets a per-frame budget; blowing it means that
plugin's output for that frame is discarded (the previous frame's output is reused), the
overrun is attributed by name, and repeated overruns unsubscribe it. Failure stays partial
and attributable instead of freezing the UI.

### Non-goals (v1)

- tmux compatibility of any kind: no `.tmux.conf`, no tpm, no `tmux` CLI surface, no
  `$TMUX`-style protocol
- Sandboxing plugins. The trust model is "you installed it" (see §6)
- Mouse support, multiple sessions, remote-machine aggregation, control-mode-equivalent
  external control API

### Success criteria for this spec's scope

- M0: `neotmux` runs an interactive shell full-screen; typing works; exit is clean
- M1: a plugin renders a clock in the status line; a plugin containing `while true do end`
  is named in a notification, dropped, and unsubscribed, while the shell and the other
  plugins keep working

## 2. Crate layout

Cargo workspace. Dependency edges point downward only; nothing depends on a crate above it.

| crate | responsibility | must not know about |
|---|---|---|
| `neotmux-proto` | wire and boundary types: IDs, geometry, grid snapshot, draw commands, client↔server messages. Types only — the sole permitted logic is `From` / `Display` style conversion. Dependencies limited to `serde` and primitives | everything else |
| `neotmux-term` | PTY spawn, VT parsing, cell grid. Wraps `portable-pty` and `vte` | pane tree, Lua, rendering |
| `neotmux-core` | session / window / pane tree, layout, event bus, state transitions | raw terminal bytes, Lua, ratatui |
| `neotmux-lua` | `mlua` bindings, plugin loading, tick dispatch, budget enforcement | ratatui, PTY |
| `neotmux` | binary: argv role dispatch, TUI client, wiring | — |

`neotmux-server` is deliberately absent in M0/M1: the binary is a single process until M3.
The split is anticipated by keeping all cross-boundary types in `neotmux-proto` from day one,
so introducing a socket later is a transport change rather than a type migration.

Rationale for `neotmux-proto` existing this early:

1. the TUI client must not link the engine — it only needs the protocol
2. an external tool (a sidecar, an alternative frontend) can depend on the published proto
   crate alone. tmux has control mode but publishes no types; this is the differentiator

`neotmux-core` must remain testable without a terminal and without Lua. That property is the
check on whether the boundaries are real.

## 3. M0 — walking skeleton

### Behaviour

`neotmux` spawns `$SHELL` on a PTY, renders it full-screen with ratatui, forwards key input,
and exits when the shell exits. One pane, no splits, no detach, no Lua.

### Components

- `neotmux-term`
  - `Pty` — `portable-pty` wrapper: spawn, resize, read/write handles
  - `Parser` — feeds bytes to `vte::Parser`, drives a `Perform` impl that mutates `Grid`
  - `Grid` — cells (char + attributes), cursor, dimensions, dirty tracking. Written by hand
    rather than taken from `vt100` so that scrollback (M4) and the Lua-facing line access are
    ours to shape. `vt100` remains the fallback if the hand-written grid proves too costly
  - `snapshot() -> GridSnapshot` (proto) — what the renderer consumes
- `neotmux` binary
  - event loop: poll PTY output, poll input, redraw on change or on a frame timer
  - renderer: `GridSnapshot` → ratatui buffer

### Data flow

```
PTY bytes → vte::Parser → Grid → GridSnapshot ──→ renderer → terminal
keypress ─────────────────────────────────────→ PTY stdin
```

### Out of scope for M0

Splits, layout, scrollback, copy-mode, detach, Lua, status line.

## 4. M1 — Lua plugin ABI

This is the milestone the project exists for; M0 is its substrate.

### 4.1 Plugin model

nvim-style, and deliberately the same shape as ttymap's Lua subsystem, which is the closest
prior art the author has run in anger:

- a plugin is a `.lua` file or `<name>/init.lua` directory under `<runtime>/lua/plugin/`,
  reachable by `require`. No registration ceremony
- activation is explicit: an `init.lua` chain (bundled runtime first, then
  `$XDG_CONFIG_HOME/neotmux/init.lua`) `require`s what it wants
- one shared Lua VM. Since plugins are trusted (§6), there is no reason to pay for
  per-plugin VMs
- identity = the require name. It is what appears in budget notifications and `tick_stats`

### 4.2 Registration surface

At least one of these must be called at load time, or the plugin is inert and says so in the
log:

- `neotmux.on_tick(fn)` — per-frame callback, receives a draw context
- `neotmux.on_event(name, fn)` — pub/sub on core events (`pane_opened`, `pane_closed`,
  `pane_output`, `focus_changed`, `resized`)
- `neotmux.keymap.set(key, fn)` — key binding
- `neotmux.status.provider(fn)` — status-line contributor

Every API that identifies a pane takes a `PaneId`, even in M1 where exactly one pane exists.
Fixing this at M1 is what stops the ABI from being reshaped by M2's splits.

### 4.3 Drawing: commands are recorded, not executed

A plugin draws by calling primitives on a draw context — `ctx:text(pane, x, y, s)`,
`ctx:rect(...)`, `ctx:line(...)`, `ctx:marker(...)` — and by returning status fragments.
Ergonomically this is the same as writing directly into the frame buffer (ttymap's `MapApi`
does exactly that today).

The difference is that **the calls append to a command buffer instead of mutating the frame.**
The host applies the buffer after the tick completes.

This is what makes the budget meaningful:

- direct mutation cannot be rolled back — a plugin interrupted mid-draw leaves half a line on
  the screen, and the frame buffer has no previous value to fall back to
- a recorded command list can be discarded wholesale, and the previous tick's list replayed

Cost is one `Vec` of commands per plugin per frame, reused across frames.

Plugins never see ratatui types. The command enum lives in `neotmux-proto`.

### 4.4 Budget and failure

- a `mlua` count hook fires every N VM instructions (N chosen so the check is cheap;
  the hook is a *sampling point*, not the unit of accounting)
- inside the hook, elapsed wall-clock time for the current plugin's tick is compared against
  the budget. Default budget: 8 ms per plugin per frame, configurable
- on overrun the hook raises a Lua error. The error unwinds that plugin's tick only
- the host then **discards this frame's command buffer for that plugin and replays the
  previous one**. The rest of the frame proceeds normally
- the overrun is recorded in `tick_stats` (per plugin: last, mean, max, overrun count) and
  surfaced through a `neotmux.notify` message naming the plugin
- after K consecutive overruns (default 5) the plugin's tick subscription is removed and a
  notification says so. Its other registrations (keymaps, event handlers) stay

`tick_stats` is queryable from Lua so a stats-overlay plugin can render it. Observability is
not a future tier; it ships with the budget, because a budget the user cannot see produces
"neotmux feels slow sometimes" bug reports.

### 4.5 Removing the ways to block

The count hook cannot interrupt C code, so the design removes the ability to enter blocking C
from Lua rather than trying to interrupt it.

Stripped from the plugin environment:

- the `io` table
- `os.execute`, `os.tmpname`, `os.remove`, `os.rename`
- `package.loadlib`, the C loader in `package.searchers`, and `package.cpath` (set empty).
  **This one matters most**: leaving `loadlib` reachable makes removing `io` pointless
- `debug` (so a plugin cannot clear its own hook)
- `load` / `loadstring` are wrapped to force `mode = "t"`, since the default `"bt"` accepts
  binary chunks and the manual states maliciously crafted bytecode can crash the interpreter

All I/O is host-provided, asynchronous, and callback-based: the call returns a handle
immediately and the completion is delivered on a later tick.

**Blocking sources that remain, and are documented rather than solved:**

1. host-provided bindings that themselves block. This is the author's discipline, not Lua's
   behaviour. Enforced by funnelling every registration through one helper and by a test that
   calls every registered function and asserts it returns within a bound
2. `__gc` finalizers — Lua disables hooks for the duration (`lgc.c` sets `L->allowhook = 0`),
   so an unbounded loop in a finalizer is uninterruptible
3. `pcall` — a plugin can swallow the budget error (`while true do pcall(...) end`). This is
   only reachable deliberately, and malice is out of the threat model (§6)

### 4.6 Where Lua runs

In the client's draw loop, single-threaded, on the same thread as rendering — the shape
ttymap uses (`dispatch` is called once per draw from the UI layer).

In M0/M1 there is one process, so this is trivially true. It is recorded here because it
constrains M3: when the server/client split lands, **Lua stays on the client**. The server
holds other people's long-running jobs; a frozen client should cost one client, not every
session on the box.

Running Lua off-thread (zellij's choice) is rejected: it exists to contain code you do not
trust, it forces grid data to be copied or locked, and it makes every plugin observe
last-frame state. None of that is worth paying for once the trust decision is made.

## 5. Error handling

| failure | handling |
|---|---|
| plugin error during load | logged with the require name, plugin inert, host continues booting |
| plugin error during tick | that plugin's command buffer discarded, previous replayed, error logged + notified |
| budget overrun | as above, plus `tick_stats` overrun counter; unsubscribe after K consecutive |
| PTY EOF | pane closed; in M0/M1 that means the process exits cleanly |
| PTY write error | logged, pane marked dead, terminal restored before any message is printed |
| panic anywhere | terminal restored (raw mode off, alternate screen left) before the panic message |

The rule: **no failure path may leave the terminal in raw mode.**

## 6. Trust model

Plugins are fully trusted. They are Lua running in the host VM with access to the host API;
they can quit the application, rebind every key, and read every pane.

This is stated positively rather than defended partially. A partial defence — an allow-list of
"safe" commands, say — does not shrink the blast radius (an infinite `on_tick` was always
available) while blocking legitimate plugins (an auto-quit script, a session manager). The
author reached the same conclusion in ttymap and deleted the allow-list that had been built
there (`76abece`).

What the budget defends against is therefore **accidents, not malice**: a development-time
infinite loop, a legitimately heavy plugin that is occasionally slow, a pathological input,
a third-party plugin with a bug. Those are the common cases. Malice is out of scope, and the
`pcall` hole is the honest consequence of that.

## 7. Testing

- `neotmux-term`: golden tests — feed byte sequences (including escape sequences from
  `vttest`-style fixtures) and assert grid contents, cursor position, and attributes
- `neotmux-core`: pure unit tests on the tree and event bus. No terminal, no Lua. If a test
  here needs either, the boundary has leaked
- `neotmux-lua`:
  - harness loads a plugin from a string, runs N ticks, asserts the resulting command buffer
  - budget: a plugin with `while true do end` is unsubscribed after K ticks; the other
    plugins in the same run still produce commands
  - environment: `io`, `os.execute`, `package.loadlib`, `debug` are absent; `load` rejects a
    binary chunk
  - non-blocking: every registered host function returns within a bound
- integration: spawn a PTY running a fixed `printf`, render, assert the frame

## 8. Open questions

1. **Should the Lua-facing types be the proto types?** Sharing them couples wire compatibility
   to ABI compatibility — breaking one breaks both. Decide before M3 introduces the socket.
   A conversion layer in `neotmux-lua` is the alternative
2. **Budget default (8 ms) and consecutive-overrun threshold (5)** are guesses. Revisit once
   a real plugin set exists
3. **Hand-written `Grid` vs `vt100`** — M0 writes its own. If escape-sequence coverage turns
   into a sink, switch and keep the Lua-facing line API as the abstraction
4. **`__gc` hole** — no mitigation planned. Documented

## 9. Roadmap (context only, not specified here)

- M2 multiplexing core: pane tree, splits, layout, focus, resize, prefix key
- M3 server/client split, detach/attach (Lua stays client-side)
- M4 scrollback and copy-mode, exposed to Lua as line access
- M5 distribution: single binary, nixpkgs, `neotmux plugin` subcommand, publish
  `neotmux-proto`, docs

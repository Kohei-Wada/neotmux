# neotmux — M0a / M1a / M1b design

Date: 2026-09-19
Status: proposed (revision 2 — incorporates a design review and a mechanism-feasibility
review; see §12 for what changed and why)
Scope: M0a (walking skeleton), M1a (ABI surface), M1b (budget and environment), M2 (pane
tree) — the milestones that make up v0.1 (§1). Later milestones are sketched in §11 for
context only.

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
plugin's output for that frame is discarded, the overrun is attributed *by name*, and repeated
overruns unsubscribe it — reversibly. Failure stays partial and attributable instead of
freezing the UI.

### The v0.1 release line

v0.1 is the line at which a plugin author can exercise the ABI. It is not the
line at which neotmux is a daily driver: detach/attach (M3) is out, and a
multiplexer without it will not attract users. That is the intended trade. The
ABI is the part that is expensive to change after publication, so it meets other
people before it freezes, and M3 — the largest structural change in the roadmap —
happens against an ABI that has already been used rather than one that has not.

v0.1 contains M1a, M1b and M2; `neotmux.pane` implemented rather than stubbed;
the default status line shipped as a plugin; `docs/plugins.md` with three working
examples; installation via `cargo install`; and a README that states detach/attach
is v0.2.

Exit criteria, each mechanically checkable:

1. No feature name (`status`, `clock`, and their kind) appears in core sources.
2. Someone given only this spec and `docs/plugins.md` can write a plugin that is
   not one of the examples.
3. A plugin that loops forever leaves the terminal usable and produces a
   name-attributed overrun in the log.
4. Every example plugin can be disabled by deleting one `require` line, with no
   change to core.

Out of v0.1, and stated as such in the README: detach/attach, scrollback and
copy-mode, mouse, multiple sessions, a plugin manager, and tmux compatibility in
any form.

### Non-goals (v1)

- tmux compatibility of any kind: no `.tmux.conf`, no tpm, no `tmux` CLI surface
- Sandboxing plugins. The trust model is "you installed it" (§8)
- Mouse, multiple sessions, remote-machine aggregation, external control API

## 2. Crate layout

Cargo workspace. Dependency edges point downward only.

| crate | responsibility | must not know about |
|---|---|---|
| `neotmux-term` | PTY spawn, VT state, cell grid. Wraps `portable-pty` and `vt100` | pane tree, Lua, rendering |
| `neotmux-core` | session / window / pane tree, layout, event bus, op buffer, state transitions. Contains the `proto` module (§2.1) | raw terminal bytes, Lua, ratatui |
| `neotmux-lua` | `mlua` bindings, plugin loading, attribution, tick dispatch, budget | ratatui, PTY |
| `neotmux` | binary: TUI client, event loop, wiring | — |

`neotmux-core` must stay testable with no terminal and no Lua. That property is the check on
whether the boundaries are real.

### 2.1 Boundary types live in `neotmux-core::proto`, not a separate crate yet

All types that cross a boundary — `SessionId` / `WindowId` / `PaneId`, geometry, grid
snapshot, draw commands — live in one module with one rule: **no behaviour beyond
what the container itself is.** Constructors, accessors, bounds checks, index arithmetic and
resizing are part of being a grid; `From` / `Display` conversions are part of being a type.
Diffing, applying, I/O, state transitions and anything that reads or decides about the rest of
the system are not, and belong in the crate that owns them.

They are *not* a separate crate in M0a–M1b. The two arguments for a crate (the TUI client
should not link the engine; external tools should be able to depend on the protocol alone)
are both M3/M5 requirements. Promoting `mod` → `crate` later is mechanical; carrying an extra
crate boundary and blanket `serde` derives through M0a is not free, and doing it now would
freeze the open question below before it has been answered.

**Open question, to be closed before M3:** are the Lua-facing types the same as the wire
types? Sharing them couples wire compatibility to ABI compatibility. The alternative is a
conversion layer in `neotmux-lua`.

## 3. M0a — walking skeleton

### Behaviour

`neotmux` spawns `$SHELL` on a PTY, renders it full-screen with ratatui, forwards key input,
exits when the shell exits. One pane, no splits, no detach, no Lua.

### Terminal state: use `vt100`, do not hand-roll

`vt100` 0.16.2 provides exactly what this project would otherwise write:

- `Parser::new(rows, cols, scrollback_len)` — scrollback is a constructor argument
- `Screen::cell(row, col)`, `Screen::rows(..)` — the per-line access the Lua API needs (M4)
- cursor position, alternate screen, wrapped-row flags, `set_size`, `contents_diff`

The earlier draft hand-rolled a grid on top of `vte` so that scrollback and line access would
"be ours to shape". Both already exist in `vt100`. The cost of the hand-rolled path is real:
`vt100`'s equivalent code is ~3,170 lines excluding tests, and it is the kind of work
(SGR sub-parameters, DECSTBM, DECSC/DECRC, `?1049` and the legacy alternate-screen variants,
ICH/DCH/IL/DL/ECH, tab stops, wide characters) that has no interesting decisions in it and
unbounded edge cases. Spending M0a there means not reaching the milestone the project exists
for.

`neotmux-term` wraps `vt100` behind `GridSnapshot` and a line-access trait. If `vt100` ever
becomes the constraint, the grid can be replaced behind that boundary — the deferred M0b in
§11. `alacritty_terminal` is the other off-the-shelf option (fuller, less stable API).

### PTY I/O: reader thread plus channel

`portable-pty` 0.9 is entirely blocking: `try_clone_reader()` and `take_writer()` return plain
`std::io::Read` / `Write`. crossterm's `event::poll` is stdin-only and exposes no way to
register an external fd in its `Poll`, so "one loop polling both" is not available.

M0a therefore runs a dedicated reader thread that `read()`s the PTY master and sends
`Vec<u8>` over a channel. The main thread owns input, VT state, Lua (from M1a) and rendering.

```
reader thread:  PTY master --read--> channel
main loop:
  drain channel -> vt100::Parser::process(bytes)
  event::poll(remaining_frame_budget) -> crossterm event -> PTY writer
  on frame deadline: (M1a: lua tick) -> render Screen -> ratatui
```

Notes fixed by the API: `take_writer()` may be called **once** (a second call fails); PTY EOF
surfaces as `EIO` translated to `Ok(0)`, so no special detection is needed.

`tokio` + `AsyncFd` + crossterm's `event-stream` is the alternative and would likely be the
better base once M3 adds a socket. It is not adopted in M0a to keep the skeleton dependency-
light; revisit at M3.

### Exit criteria (machine-checkable)

- A PTY harness writes `printf 'X'`; after render, the frame buffer cell at (0,0) is `X`
- The child exits 0 and the process exits 0
- After exit, `termios` has `echo` and `icanon` restored
- A test that panics mid-render still restores the terminal (§7)

## 4. M1a — the ABI surface

This is the milestone the project exists for. It deliberately contains no budget: the budget
(M1b) is worthless if the ABI underneath it is wrong, and the ABI is what is expensive to
change after publication.

### 4.1 There is no plugin model

The host has no concept of "plugin". It provides an API (§4.3–§4.9) and a loading
convention (this section); a plugin is a bundle of API calls, and it is a plugin only as a
consequence of being loaded through that convention, not because the host recognises it as
one. This is nvim's own design: nvim-core has no plugin type either, `runtimepath` plus
`plugin/` is a loading convention, and attribution is by **source location** — what
`:verbose` reports as "Last set from `<file>`" — not a host-tracked plugin registry.

Giving the host a first-class plugin type would force it to have opinions that belong to
whoever is writing plugins: what a plugin is named, how its files are laid out, whether it is
one file or many. None of that is the host's business. The loading convention below is
sufficient, and everything the host actually needs to do — attribute a callback, remove a
failed unit's registrations, order a draw buffer — is answerable from **where the code that
made the call was written**, which §4.2 calls its source.

The convention itself, nvim-style: a `.lua` file or `<name>/init.lua` under
`<runtime>/lua/plugin/`, reachable by `require`; no registration ceremony; activation is an
`init.lua` chain that `require`s what it wants; one shared Lua VM.

This much is the shape ttymap uses and is known to work. **The budget and environment work in
M1b has no precedent in ttymap** — a grep of `ttymap-lua/` finds no use of `set_hook`,
`HookTriggers`, `StdLib` restriction, `loadlib` or `cpath`; the only `package.searchers` use
is appending a runtime-path searcher. M1b is new development, not a port.

**Runtime layers**, resolved in order, each contributing `<layer>/lua/`:

1. `$NEOTMUX_RUNTIME` (colon-separated), if set
2. `$XDG_CONFIG_HOME/neotmux/`
3. the bundled runtime, located relative to the executable
4. `/usr/share/neotmux/`

No compile-time developer path is baked in. **Zero resolvable layers is a normal startup**
with no plugins, not an error — a multiplexer that refuses to start because its config is
broken has failed at its only job.

### 4.2 Attribution is by source, not by identity

Every budget notification, log line and unsubscribe decision names where a callback came
from, so the host must know that even though it tracks no plugin registry (§4.1). With a
single VM and a plain `package.path` searcher it does not by default — ttymap removed its
per-plugin `host_tag` in #240 and its docs now state "no host-side plugin attribution".
Stripping `debug` (§5.2) also removes the Lua-side fallback. So attribution is designed in,
not assumed, and it is designed around **source location**, exactly as nvim's `:verbose`
answers "who set this" by naming a file, not by looking up an entry in a plugin table:

- **the single rule: the owner of an API call is the chunk where the calling code was
  written.** Never where it happens to run from, never host bookkeeping. **The host keeps no
  current-owner stack** — there is nothing to push, nothing to pop, and nothing that can be
  out of date by the time a call arrives
- for a registration there is a callback to inspect, so the owner is wherever
  `Function::info().source` says that closure was written. `neotmux.on_tick`, `on_event`,
  `on_key` and `keymap.set` all read it. A source location is lexical, so it is
  correct regardless of when or from where the callback is later called. This is what makes
  the idiom every plugin author reaches for coming from nvim,
  `local m = require('plugin.clock'); m.setup{}`, attribute correctly with no special case:
  `setup`'s callback still carries `clock.lua` as its source, and so does a second `require`
  that hits `package.loaded` and never re-enters the searcher
- for a direct call with no callback — `neotmux.log.info("x")`, `neotmux.quit()`,
  `neotmux.notify(...)` — the host reads the calling Lua frame with `Lua::inspect_stack`
  (level 0 is the host binding itself, a C function; the owner is the first frame above it
  that is not a C frame, because `pcall`, `coroutine.wrap` and a callback invoked by
  `table.sort` or `string.gsub` each sit between the binding and the chunk).
  Both paths end in the same step: intern an `@<path>` source, or fall to `ANONYMOUS`
- the two paths agreeing is the point. A `setup` function written in `clock.lua` that both
  registers a provider and logs a line produces one owner, `plugin/clock`, for both — where
  a stack would have said `plugin/clock` for the registration and `init` for the log
- the consequence, accepted rather than worked around: a plugin callback that calls a helper
  living in a library chunk, which then calls `neotmux.log`, attributes the log to the
  **library**. That is the same rule that already makes a library which registers a callback
  own that callback. There is no "walk up until you leave the library" heuristic
- the searcher still **interns** every chunk it loads, plugin or library alike, at load time.
  That is what allocates `SourceId`s in load order, which the draw ordering guarantee (§4.5)
  rests on. It does not wrap the chunk
- a source is a host-side `SourceId` with a **display name** derived from its path: the
  runtime layer prefix and the `.lua` suffix are stripped, so `@<layer>/lua/plugin/clock.lua`
  displays as `plugin/clock`, `@<layer>/lua/neotmux/config.lua` as `neotmux/config`, and
  `@<layer>/init.lua` as `init`. `init.lua` is not a reserved identity; it is a source like any
  other, interned like every other chunk. Two
  layers providing the same relative path collide on this display name on purpose — they are
  the same logical file — and the full source string remains available for anything that needs
  to disambiguate
- `require('plugin.foo')` and `require('plugin.foo.init')` can resolve to the same file
  (`?.lua` and `?/init.lua` are both on `package.path`); the searcher's chunk name is built
  from the *resolved file path*, not from the string passed to `require`, so either spelling
  interns the same `SourceId` instead of splitting one file's registrations across two. A plugin split across several files (`plugin/clock/init.lua` and
  `plugin/clock/util.lua`) does *not* get this collapsing — those are genuinely different
  files and so genuinely different sources, with independent registrations, draw buffers and
  log tags. Nothing in the host groups them back into one identity; a plugin author who wants
  that today keeps state and registration in one file
- a callback with no real chunk behind it — a native (Rust) function, a `loadstring`d chunk —
  has no source to intern; it is `ANONYMOUS`, the one sentinel the host keeps, reserved for
  exactly this "genuinely no source" case rather than for `init.lua`. A call reached with no
  Lua frame below it at all is the same case

Because `debug` is unavailable, Lua-level tracebacks for error messages are produced by the
host: the error path attaches the captured `SourceId` and the chunk name rather than asking
Lua where it is.

### 4.3 Registration surface — everything returns a handle

```lua
local h = neotmux.on_tick(fn)                    -- per-frame draw callback
local h = neotmux.on_event(name, fn)             -- pane_output / resized
local h = neotmux.on_key(fn)                     -- every key, before the bindings
local h = neotmux.keymap.set(key, fn)            -- key binding
h:remove()                                       -- idempotent
```

**Every registration returns a disposable handle and `:remove()` is idempotent.** The host
also keeps the set of registrations per `SourceId`. Reload is not implemented in M1a, but
reload is impossible to add later without breaking the ABI unless this shape is fixed now
(ttymap #241 is still open for exactly this reason). Freezing the shape is cheap today.

The event list is exactly the events the host fires. An event the host defines but never
dispatches is worse than a missing one: a plugin subscribing to it waits forever and finds
out only by experiment. The pane-tree events belong to M2 (§6), which is where they can
first be fired.

Every API that names a pane takes a `PaneId`, even though M1a has exactly one pane. This is
what stops M2's splits from reshaping the ABI.

### 4.4 Key handling is part of the ABI

In a multiplexer nearly every byte must reach the PTY, which makes key routing the most
contended resource in the system. The contract, fixed in M1a:

- a keymap callback returns `true` to consume the key, and `false` or `nil` to let it fall
  through to the PTY. **The return value is ABI** — introducing it in M2 would break every
  plugin written against M1a
- **no key is reserved by the host.** There is no prefix key in the host: §6's prefix is an
  ordinary consumer of this API. A host that reserved the key would have decided in advance
  that the plugin cannot be written the ordinary way
- two plugins binding the same key is an error at registration time: the second registration
  fails, returns `nil` plus a message, and the conflict is logged with both source names.
  Last-wins is rejected because it makes load order load-bearing and silent
- **the two failures of `keymap.set` are not the same kind of failure, and do not report the
  same way.** A spec the host cannot parse is a programming error, like every other malformed
  argument in the ABI, and **raises** — `pane.split`, `notify`, `ctx:text` and `opt.<key>` all
  do. Losing a conflict is not a programming error: the caller wrote a legal spec and another
  source happened to get there first, so it **returns `nil` plus a message** for the caller to
  handle

**Normalisation canonicalises the modifiers, not the key.** `C-b`, `c-b` and `Ctrl-b` are one
spec; `x` and `X` are two, because the host reports a character key as the character the
terminal produced. For the same reason `S-` does not combine with a single character — such a
spec could never be dispatched — and is refused at registration, naming the uppercase
character to bind instead. `S-` with a named key (`S-Tab`) is ordinary.

A key is dispatched in four steps:

1. every `neotmux.on_key` callback runs, each receiving the normalised key spec as a string
   — the same form `keymap.set` takes, so a plugin can compare the two;
2. the first callback to return `true` consumes the key: later `on_key` callbacks do not run
   and the keymap lookup does not happen;
3. otherwise the `keymap.set` binding for that spec runs, with the semantics above;
4. if nothing consumed the key, it goes to the PTY.

`on_key` exists because `keymap.set` binds one callback per spec, so a plugin can only see
keys it has already bound. A prefix is modal — it must see the key *after* its own, whatever
that key is — and holding a permanent global binding on every command key would both fail
outside prefix mode and deny those keys to every other plugin through the conflict path. It
is a notification with a veto, and nothing more: the host has no mode, no mode stack and no
prefix concept, and everything modal is the plugin's own state.

**`on_key` and `on_event` callbacks run in registration order.** That is not the same thing as
load order, and the two are not interchangeable: registration order is the order the
`on_key` calls happened in, which for anything registered from inside a tick or a keymap is
not where its chunk sat in the init chain.

Tick callbacks are the exception, and deliberately: they are dispatched grouped by source, in
load order, because the frame commits per source (§4.5). Within one source its own ticks run
in registration order. Drawing then composes sources in load order as well, so what lands on
screen never depends on when a callback happened to be registered.

There is no priority and no ordering knob for any of them.

A callback that raises is logged against its own source and treated as not consuming, so one
broken handler cannot swallow the keyboard.

### 4.5 Drawing: commands are recorded, not executed

A plugin draws through a context object:

```lua
neotmux.on_tick(function(ctx)
  local w, h = ctx:size()                 -- the surface it is drawing into
  local rect = ctx:pane_rect(pane_id)
  ctx:text(pane_id, x, y, "hello")
  ctx:rect(...) ctx:line(...) ctx:marker(...)
  ctx:status("12:34", { fg = "#ffffff" })  -- a fragment of the status line
end)
```

The status line is contributed the same way as anything else drawn: `ctx:status(text,
colours)` appends a fragment to the calling source's buffer for this frame. There is no
separate status registration — a source that stops calling it stops contributing, and the
host shows the status row only while some fragment exists.

`ctx:size()` and `ctx:pane_rect()` exist because without them a status plugin cannot know its
own width — the clock in the exit criteria only worked because a clock ignores width.

Calls **append to a command buffer; they do not mutate the frame.** The host applies the
buffer after the tick. Direct mutation (what ttymap's `MapApi` does today) cannot be rolled
back: a plugin interrupted mid-draw leaves half a line on screen and there is no previous
value to fall back to. A recorded list can be discarded wholesale and the previous one
replayed. Ergonomics are identical; cost is a freshly allocated `Vec` of commands per source
per frame — nothing is pooled or reused, and if that ever shows up in a profile it is a change
behind this interface, not a change to it.

**The unit of commit is the source, not the callback.** A source may register more than one
`on_tick`; its frame is the union of what all of them drew, committed once after the last one
returns. If any of them raises, the whole union is discarded and the source's previous frame
stands — a partial union is not a frame, and committing per callback would make each callback
overwrite the one before it.

**Drawing composes sources in load order**, so a source loaded later draws over one loaded
earlier. This is per source, not per callback and not per registration: the order a source's
own tick callbacks were registered in decides nothing about what is on top, because they all
land in that source's one buffer.

**Replay validity.** Commands carry absolute coordinates and `PaneId`s, so replaying a stale
buffer after a resize or a pane close would draw at wrong coordinates or into a dead pane.
Each buffer is stamped with an **epoch** = (grid dimensions, set of live pane ids, focused
pane). A stale buffer is replayed only if its epoch matches the current one; otherwise it is
discarded silently and the plugin contributes nothing this frame. There is no previous buffer
on the first frame, which is the same case.

Plugins never see ratatui types. The command enum lives in `neotmux-core::proto`.

### 4.6 Reentrancy: one op buffer, drained in one place

All plugin→host intent (draw commands, requests to open/close/focus, config changes) goes
into a single op buffer; the host drains it at one point in the frame. Host state is never
handed to Lua as a live mutable borrow, and **host bindings never re-enter Lua**.

This is ttymap #294 (open): "register from anywhere" plus `Rc<RefCell<...>>` host state
equals runtime `BorrowMutError`. The op-buffer discipline is the conclusion that issue
reaches; adopting it up front costs nothing.

### 4.7 Configuration

One idiom, stated so that three are not invented:

- a plugin that has settings ships a **config holder** module at `<runtime>/lua/neotmux/<name>.lua`
  that returns a table of defaults
- `init.lua` `require`s it and mutates fields before the plugin is `require`d
- the plugin reads the holder at load time

Host settings (budget, thresholds) use `neotmux.opt.<name> = value`, applied at `init.lua`
time. The prefix is not among them — it belongs to the plugin that implements it (§6).

### 4.8 Resize ordering

Fixed, because otherwise plugins draw against last frame's dimensions:

1. terminal resize observed by the client
2. `vt100::Screen::set_size` and PTY `resize()` applied
3. pane rects recomputed
4. `resized` event dispatched
5. tick dispatched — `ctx:size()` already returns the new values
6. render

Command buffers from before step 3 fail the epoch check and are not replayed.

### 4.9 Logging and notification

- log destination: `$XDG_STATE_HOME/neotmux/neotmux.log`; `--log <path>` overrides it. There
  is no level flag: `neotmux.log.{info,warn,error}` all reach the file
- log initialisation failure is reported on stderr **before raw mode is entered**, then
  startup continues. A full-screen TUI cannot use stderr afterwards (ttymap #226 is the silent
  version of this bug)
- `neotmux.log.{info,warn,error}` for plugins, tagged with the source of the chunk the call
  was written in, recovered with `Lua::inspect_stack` (§4.2). A call made from a
  post-`require` `setup()` is tagged with that plugin, not with `init`
- notifications (budget overruns, unsubscribes, key conflicts) render through a **host-owned**
  surface, not a plugin. Every notification raised during the session is also kept and written
  to stderr once the terminal has been restored at exit, so one that expired, scrolled past or
  was never drawn is not left in the log file alone

### 4.10 Failure handling at load

**The guarantee: one entry in the init chain failing to load does not abort the rest of the
config.** A failed entry is inert — the registrations *it owns* are withdrawn — it is
reported once by name, and loading continues.

**The limit, accepted rather than fixed:** ownership is lexical (§4.2), so a callback the
failing chunk got from a library is owned by the library, not by the chunk that registered it,
and it survives the rollback and runs afterwards. Withdrawing it too would mean tracking
"registered during this load" alongside "owned by" — a second notion of ownership, and the
first thing that would have to be right for every later feature that asks who owns what. The
case is not worth that, so the rollback covers the failing source's own registrations and no
more. A test pins the behaviour so it stays a decision.

| failure | handling |
|---|---|
| `init.lua` raises | log + notify, continue with whatever loaded before the error |
| `init.lua` absent | normal startup, no plugins |
| no runtime layer resolves | normal startup, no plugins |
| an entry in the init chain fails to load | that entry is inert; its partial registrations are removed via its handle set; others unaffected |

The mechanism is the **load convention** (§4.1), not `require`, and it is keyed on nesting
depth rather than on the module name — the host holds no opinion about which names are
plugins:

- the isolation is armed only while the `init.lua` chain is running. A `require` reached at
  runtime, out of a tick callback or a keymap, is an ordinary `require` and its error
  propagates to whoever called it
- while armed, a `require` with no other `require` below it on the stack is a
  load-convention load: on failure its source's registrations are removed, the failure is
  recorded against that source, and it evaluates to `nil`
- a `require` nested inside another `require` propagates its error normally. A plugin's own
  missing dependency is that plugin's error: it aborts that plugin, is attributed to it, and
  surfaces as the outer load's recorded failure
- a module that does not resolve to a file at all — a typo, a plugin that is not installed —
  never ran and owns no registrations, so nothing is removed and the failure is recorded
  under the name that was asked for

This is nvim's shape: isolation lives in the thing that iterates what to load, not in
`require`.

### 4.11 Exit criteria (machine-checkable)

A fixture plugin **set**, not one plugin — the point is to exercise the ABI:

- `clock` — a status-line contributor: an `on_tick` that calls `ctx:status()` with a string
  sized from `ctx:size()`; asserted to produce a non-empty command buffer every frame
- `watcher` — consumes a payload-carrying `pane_output` event; asserted to observe bytes a
  harness wrote to the PTY
- `binder` — `keymap.set` returning `true` for one key and `false` for another; asserted that
  the consumed key never reaches the PTY and the other does
- `prefix` — a real two-key prefix: it binds nothing, and uses `on_key` to swallow its own
  key and then the next one; asserted that neither reaches the PTY while an unrelated key does
- `conflicting` — binds the same key as `binder`; asserted that registration returns `nil`,
  both names appear in the log, and `binder`'s binding still works
- `configured` — reads a config holder that `init.lua` mutated; asserted to see the mutated
  value
- `remover` — registers then `:remove()`s twice; asserted idempotent and that no callback runs
  afterwards
- a plugin calling the M2 pane/split API (§6) against a stub, to freeze those signatures now

Plus: every callback's attributed source (§4.2) is the chunk the callback was written in,
including one registered from inside another plugin's dispatch. There is no `PluginId`: the
host tracks `SourceId`s, and a display name such as `plugin/clock` is derived from the chunk
path.

## 5. M1b — budget and environment

### 5.1 Budget

- **`Lua::set_global_hook`, not `Lua::set_hook`.** mlua's per-thread hook proc looks the
  current thread up in a weak registry table and, when it is missing, *silently removes the
  hook* (`state/raw.rs`). Lua 5.4's `lua_newthread` inherits the C hook pointer but not that
  registration, so `coroutine.wrap(function() while true do end end)()` would disable the
  budget in three lines. The global hook reads the callback off the host's extra data and has
  no thread dependency; `create_thread` sets `HookKind::Global` explicitly
- `HookTriggers::new().every_nth_instruction(n)`; `n` tuned by measurement. The hook is a
  **sampling point**, not the accounting unit: it reads `Instant` and compares the current
  plugin's elapsed tick time against the budget
- default budget 8 ms per plugin per frame, `neotmux.opt.tick_budget_ms`
- on overrun the hook returns `Err`, which mlua converts to a real Lua error
  (`callback_error_ext` → `lua_error`), unwinding to the host's protected call — so the tick,
  and only that tick, is rolled back. `luaD_pcall` restores `allowhook`, so the hook survives
  and the next frame is measured normally
- the host discards that plugin's command buffer and replays the previous one if its epoch
  matches (§4.5)
- the hook performs no Lua calls: `luaD_hook` clears `allowhook` for its duration, so anything
  Lua-side inside it runs unmeasured. Rust only
- `VmState::Yield` is not used: the tick is called from the draw loop and is not yieldable

### 5.2 Environment

Construct with the **safe** constructor (`Lua::new_with(libs, opts)`), never `unsafe_new*`.
That alone disables `package.loadlib` and replaces the C searcher — mlua calls
`disable_c_modules()` whenever `StdLib::PACKAGE` is loaded. `StdLib::DEBUG` is rejected by the
safe constructor outright, so `debug` needs no manual removal.

On top of that, the host:

- omits `StdLib::IO` entirely
- keeps `StdLib::OS` but `raw_set`s `execute`, `exit`, `tmpname`, `remove`, `rename`,
  `setlocale`, `getenv` to `nil`. Dropping the whole library would take `os.time` / `os.clock`
  / `os.date` with it and the clock fixture could not be written
- removes `dofile` and `loadfile` — both read a file with `mode = NULL`, i.e. accept binary
  chunks, which is the same hole `io` was removed for
- empties `package.cpath` and removes any remaining C entries from `package.searchers`, then
  appends the runtime-path searcher (order matters: remove, then append)
- wraps the global `load` in Lua so the `mode` argument is forced to `"t"` (the Lua default is
  `"bt"`, and the manual warns that crafted bytecode can crash the interpreter). Wrapping in
  Lua rather than Rust keeps the reader-function form of `load` working. `loadstring` does not
  exist in 5.4. The wrapper forwards the environment argument only when the caller actually
  supplied one: Lua tells an absent 4th argument from an explicit `nil`, and passing `nil`
  leaves the chunk with a nil `_ENV` and no reachable global
- loads every plugin chunk with `Chunk::set_mode(ChunkMode::Text)`. mlua's automatic detection
  loads a binary chunk if it sees the signature, so the host's own load path needs this too

All host I/O bindings are asynchronous: the call returns a handle and completion arrives on a
later tick. Bindings are registered through a single helper that records a sample invocation,
which is what makes §9's "every host function returns promptly" test possible to write.

### 5.3 Overrun policy, and why it is reversible

- overruns are counted per source; the counter resets on any tick that completes within budget
- after K consecutive overruns (default 5, `neotmux.opt.tick_overrun_limit`) the **tick**
  subscription is removed and a notification names the source. Its keymaps and event handlers
  are untouched, and its last frame is dropped rather than replayed (§4.5)
- **unsubscribing is not terminal.** `neotmux.resubscribe(name)` restores it, taking the
  source name the notification reported. There is no `neotmux.plugin` namespace — the host has
  no plugin concept (§4.1) — and there is no host reload function: re-running a chunk is
  `package.loaded[m] = nil; require(m)`, written in Lua, because the host has nothing to add
  to it. Without a recovery path the only cure is restarting neotmux, which kills every shell
  the user is running — paying for a plugin bug with the entire reason the program exists

### 5.4 What the budget does not catch

Documented, not solved:

1. **a single long C call** — `string.rep(s, 1e9)`, a pathological `string.find` backtrack, a
   huge `table.concat`. The hook only fires on Lua instruction boundaries, so one C call can
   overrun the budget arbitrarily, or exhaust memory before it returns
2. **`__gc` finalizers** — Lua clears `allowhook` for their duration (`lgc.c`), so a loop in a
   finalizer is uninterruptible
3. **`pcall`** — `while true do pcall(f) end` swallows the budget error. This *is* closable:
   with `LuaOptions { catch_rust_panics: false }` mlua replaces global `pcall`/`xpcall` with
   variants that re-raise Rust panics, so panicking in the hook instead of returning `Err`
   cannot be caught. It is **not adopted**, because it requires wrapping ticks in
   `catch_unwind` and collides with `ratatui::init()`'s panic hook, which restores the terminal
   before the unwind we intended to catch. `coroutine.resume` is not replaced either, so the
   hole would not fully close. Deliberate loops are malice, and malice is out of scope (§8)
4. **host bindings that block** — the host's own discipline, covered by §9's test

### 5.5 Exit criteria (machine-checkable)

- plugin A (well-behaved) produces a non-empty command buffer on every frame while plugin B
  contains `while true do end`; after K frames B is absent from the tick subscriber list,
  exactly one notification names B, A is unaffected, and the PTY child is still alive
- the same with B wrapped in `coroutine.wrap` — the global-hook requirement is what this tests
- `io`, `os.execute`, `dofile`, `loadfile`, `package.loadlib`, `debug` are all `nil` in the
  plugin environment; `package.cpath` is empty
- `load` rejects a binary chunk; the host's plugin loader rejects one too
- `neotmux.resubscribe(name)` brings B back and it is dropped again after K frames

## 6. M2 — pane tree

### The model

A window owns a binary tree. A leaf is a pane: one PTY, one `vt100` parser, one rect.
An internal node is a split with an orientation (horizontal or vertical), exactly two
children, and the fraction of its own rect that goes to the first child.

`split(dir)` replaces the focused leaf with an internal node whose children are the old
pane and a new one, oriented by `dir` and starting at 0.5. Closing a pane replaces its
parent with its sibling: the sibling inherits the parent's rect, so closing never leaves
a hole and never needs a rebalance pass.

This is tmux's model and it has tmux's weakness — three equal columns are
`Row(A, Row(B, C))` and stay equal only by accident of the fractions. That is accepted
(§1, v0.1 scope). What is not accepted is foreclosing the alternative: **rect assignment
happens in exactly one function**, `layout::assign(&Tree, Rect) -> Vec<(PaneId, Rect)>`,
with no other code in the tree module computing geometry. Making layout replaceable from
Lua later is then "let a registered callback stand in for that function", not a
restructuring.

### Where it lives

`neotmux-core`, and it must stay testable with no terminal and no Lua (§2). The tree is a
pure data structure over `PaneId`s; it does not own PTYs. The binary owns the map from
`PaneId` to its PTY handle and `vt100` parser, and asks core for the rects.

### Rounding and minimum sizes

`assign` works in integer cells and must be total: every cell of the window is covered by
exactly one pane rect, with no gaps and no overlap, at every size including sizes too
small to honour the fractions. Rounding error goes to the first child. A split whose
children would fall below one cell clamps rather than producing an empty rect, because
`vt100` and the PTY both reject a zero dimension (M0a).

### Focus

Focus is a `PaneId` held beside the tree, not a flag inside it, so that "the focused pane
closed" is one assignment rather than a tree walk. When the focused pane closes, focus
moves to the sibling subtree's first leaf in traversal order.

### The prefix key is not special

There is no hardcoded prefix. The prefix is a plugin: an `on_key` callback (§4.4) that
swallows one key, sets a flag, and consumes the next key whatever it is. That is the test of
whether the key API from M1a is real — if the prefix cannot be written as a plugin, the API
is missing something, and finding that out is the point of building it this way. It is what
`on_key` was added for: `keymap.set` alone cannot express it, because the key after the
prefix is one the plugin has not bound.

M2 also introduces the pane-tree events — `pane_opened`, `pane_closed` and
`focus_changed` — deliberately held back from M1a because nothing there could fire them.

`neotmux.pane` gives Lua `list()`, `current()`, `split(dir)` and `focus(id)`, the
signatures frozen in M1a. Every one of them is an op on the buffer (§4.6), applied after
the tick, never a synchronous mutation from inside a callback.

### Exit criteria

1. Splitting, focusing, closing and resizing work, driven from Lua, with no pane-related
   key handling in core.
2. `assign` is covered by a property test: for a tree of any shape and a rect of any size
   down to 1x1, the output rects tile the input exactly.
3. The prefix key ships as a plugin file, and deleting its `require` line leaves a working
   multiplexer with no prefix.
4. Closing the last pane exits, and the terminal is restored (the M0a guarantee still
   holds with a tree in place).

## 7. Error handling

| failure | handling |
|---|---|
| plugin error during tick | that plugin's buffer discarded, previous replayed if epoch matches, error logged and notified with the plugin name |
| budget overrun | as above, plus counter; unsubscribe after K consecutive; reversible (§5.3) |
| key conflict at registration | second registration fails with `nil` + message, both names logged |
| PTY EOF (`Ok(0)`) | pane closed; in M0a–M1b that means clean exit |
| PTY write error | logged, pane marked dead, terminal restored before anything is printed |
| panic anywhere | terminal restored before the panic message |

**Invariant: no failure path may leave the terminal in raw mode.** `ratatui::init()` installs a
panic hook that restores first; the test suite includes a plugin that panics the host and
asserts `termios` afterwards, because this is the only failure whose blast radius is the
user's shell session.

**Known hole, left open by design in M1a: a tick callback that never returns.** `dispatch_tick`
calls straight into the registered Lua function with no time limit, so `while true do end` in
one plugin's tick blocks the frame loop forever. `shutdown.raised()` is checked once per frame
(§2), so it becomes unreachable, and SIGTERM cannot be delivered — the terminal stays in the
alternate screen until SIGKILL, which this invariant cannot cover without one. This is not an
oversight to patch here: it is exactly the failure mode M1b's instruction-count budget (§5.1)
exists to close, by making the hook return `Err` before the frame loop can hang. Inventing a
second interruption mechanism in M1a would duplicate that work and is explicitly out of scope.

## 8. Trust model

Plugins are fully trusted: Lua in the host VM, able to quit the application, rebind every key,
read every pane.

Stated positively rather than defended partially. A partial defence — an allow-list of "safe"
commands — does not shrink the blast radius (an infinite `on_tick` was always available) while
blocking legitimate plugins (an auto-quit script, a session manager). The author reached this
conclusion in ttymap and deleted the allow-list built there (`76abece`).

The budget therefore defends against **accidents, not malice**: a development-time infinite
loop, a legitimately heavy plugin that is occasionally slow, a pathological input, a
third-party plugin with a bug. The `pcall` hole (§5.4) is the honest consequence.

The same applies to the `neotmux` table seal: it catches `neotmux.on_tick = ...` by mistake,
not a plugin determined to bypass it — `rawset(neotmux, 'on_tick', ...)` still reaches the
real table, and closing that would cost every plugin its legitimate use of `rawset` to stop a
deliberate act the trust model already permits.

## 9. Testing

- `neotmux-term`: golden tests feeding byte sequences, asserting screen contents, cursor,
  attributes, alternate-screen transitions
- `neotmux-core`: pure unit tests on tree, event bus, op buffer, epoch computation. No
  terminal, no Lua — if a test here needs either, the boundary has leaked
- `neotmux-lua`:
  - harness loads plugins from strings, runs N frames, asserts command buffers and attribution
  - the fixture plugin set of §4.11 as one integration test
  - budget tests of §5.5, including the coroutine variant
  - environment tests of §5.5
  - **non-blocking**: iterate the host-binding registry (§5.2) and assert each sample
    invocation returns within a bound, with I/O mocked
- integration: PTY harness writes known bytes, asserts rendered frame; panic test asserts
  terminal restoration

## 10. Risks

- **the ABI is wrong even though M1a passes.** Mitigated by making the exit criteria a plugin
  set that exercises attribution, handles, key consumption, config and `PaneId`, and by
  freezing M2 signatures against a stub. This is cheap now and expensive after publication
- **M0a overruns.** Mitigated by taking `vt100` instead of writing a VT implementation
- **budget mechanism is new work**, with no ttymap precedent to copy (§4.1). M1b is separated
  from M1a so that a hard M1b does not block the ABI

## 11. Roadmap (context only)

- **M0b** hand-rolled grid behind `neotmux-term`'s boundary — only if `vt100` becomes the
  constraint
- **M3** server/client split, detach/attach. **Lua stays on the client**: the server holds
  other people's long-running jobs, so a frozen client should cost one client, not every
  session. Promote `proto` to a crate here and close §2.1's open question
- **M4** scrollback and copy-mode, exposed to Lua as line access
- **M5** distribution: single binary, nixpkgs, `neotmux plugin` subcommand, publish `proto`

## 12. Revision log

Revision 2 changed, relative to the first draft:

- **`set_global_hook`** replaces `set_hook` (§5.1) — the per-thread hook is silently removed
  inside coroutines, which would have made the budget defeatable in three lines
- **`vt100` replaces a hand-rolled grid** in M0a (§3) — the reasons given for hand-rolling
  (scrollback, line access) are already provided by `vt100`, and the hand-rolled path is
  ~3,000 lines of edge cases standing between the project and its actual goal
- **`neotmux-proto` demoted to a module** (§2.1) — the arguments for a crate are M3/M5
  arguments
- **attribution promoted to a designed mechanism** (§4.2) — it was assumed ("identity = the
  require name") in a design that also strips `debug`; ttymap removed exactly this capability
  in #240
- **handles, key-consumption semantics, `ctx:size()`, replay epochs, resize ordering,
  reentrancy, config idiom, logging, broken-`init.lua` behaviour** added (§4.3–§4.10) — all
  are ABI or startup behaviour that cannot be added later without breaking plugins
- **unsubscribe made reversible** (§5.3) — it was a terminal state whose only cure was a
  restart, i.e. killing every shell
- **M1 split into M1a (ABI) and M1b (budget)**, and exit criteria rewritten as a fixture
  plugin set with machine-checkable assertions
- **environment section corrected** (§5.2) — mlua's safe constructor already disables
  `loadlib` and the C searcher; `dofile`/`loadfile`/`os.exit` were missing; dropping
  `StdLib::OS` wholesale would have broken the clock fixture; the host's own loader needs
  `ChunkMode::Text`; `loadstring` does not exist in 5.4
- **ttymap's precedent scoped** (§4.1) — it covers the plugin model and draw API only; it
  contains no hook, sandbox or budget code
- **single long C calls** added to what the budget cannot catch (§5.4)
- **PTY threading decided** (§3) — blocking reader thread plus channel, because crossterm
  cannot poll an external fd

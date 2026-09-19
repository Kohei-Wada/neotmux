# Writing a neotmux plugin

neotmux has no plugin system. It has an API, and a convention for where Lua files live. A
plugin is a file that calls the API; it is a plugin only because you call it one. Nothing
registers it, nothing declares it, and the program never learns the word.

That is not a slogan — it is the reason things work the way they do below. There is no manifest
because there is nothing to declare to. There is no enable/disable because loading a file is
what "enabled" means. There is no plugin name because the thing neotmux knows about is the
*chunk your code was written in*, which it names after the file's path.

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
neotmux.on_tick(function(ctx)
  ctx:status('hello', { fg = '#00ff88' })
end)
```

`~/.config/neotmux/init.lua`:

```lua
require('plugin.hello')
```

Start neotmux and `hello` is in the status line. Delete the `require` line and it is not. That
is the whole lifecycle.

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
| `neotmux.keymap.set(spec, fn)` | for that one key; return `true` to consume it |

Events that fire today: `resized` (`rows`, `cols`) and `pane_output` (`pane`, `bytes`). There
are no others — if you subscribe to a name nothing sends, nothing happens, silently.

`keymap.set` returns `nil, message` when another source already holds that key. It *raises*
when the spec itself is unparseable: losing a race is something to handle, a typo is a bug.

Key specs are `C-`, `M-`, `S-` prefixes in any order plus a key: `C-b`, `M-Enter`, `F5`, `x`.
`x` and `X` are different keys, because that is what the terminal reports. `S-` does not
combine with a single character for the same reason.

### Drawing

Drawing happens only from a tick, through the `ctx` your callback is handed. It is
immediate-mode: **what you draw this tick is your entire output for this frame.** Draw nothing
and you show nothing. There is no clear call because you never need one.

```lua
neotmux.on_tick(function(ctx)
  local size = ctx:size()                      -- { rows = , cols = }
  ctx:status('text', { fg = '#ffffff', bg = 1 })
  ctx:text(nil, x, y, 'text', { fg = 2 })      -- nil pane = the whole window
  ctx:rect(nil, x, y, w, h, { bg = '#202020' })
  ctx:line(nil, x1, y1, x2, y2, { fg = 4 })    -- no bg: it paints single cells
  local r = ctx:pane_rect(pane_id)             -- { x = , y = , w = , h = } or nil
end)
```

Colours are a palette index `0..255` or a `"#rrggbb"` string. Omit the table and you get the
terminal's defaults.

`ctx:size()` is the area available to the shell, so it shrinks by a row the moment any source
contributes a status fragment. Read it every tick rather than caching it.

**`ctx:rect` with no colours clears the area.** It is not a no-op — default background is a
colour like any other, and painting it is how you erase.

### Panes

```lua
neotmux.pane.list()      -- { id, ... }
neotmux.pane.current()   -- id, or nil
neotmux.pane.split('horizontal' | 'vertical')
neotmux.pane.focus(id)
```

`split` and `focus` do not take effect while your callback is running. They are queued and
applied after the tick, so `list()` returns the same thing immediately after a `split` as
immediately before. This is deliberate: a callback that could reshape the world underneath the
callbacks running after it would make frame composition depend on dispatch order.

A frame applies at most three splits. Each one costs the host a fork, an exec and a terminal,
none of it inside the tick your budget was measured against, so the number of them one frame
can be asked for is bounded; the fourth and beyond are refused and logged against you. There is
no way to close or resize a pane from Lua in v0.1.

`pane_opened`, `pane_closed` and `focus_changed` are dispatched against the layout as it is
after the change, so a handler's `pane.list()` and `pane.current()` describe the event it is
being told about.

A terminal too small to hold the tree hides the panes that no longer fit rather than closing
or shrinking them, and hiding a pane does not unfocus it: keys still go to it, so you are
typing blind until the terminal grows back. The terminal cursor is hidden for as long as that
lasts. It has nowhere honest to sit — the panes tile the surface exactly, so any visible cell
belongs to a pane that is not getting the keys — and no cursor is a better answer than one
pointing at the wrong pane.

### Everything else

```lua
neotmux.log.info(text)    -- also .warn, .error
neotmux.notify(level, text)  -- 'info' | 'warn' | 'error'; shown on screen and logged
neotmux.quit()
neotmux.opt.tick_budget_ms = 8     -- per source per frame; 0 turns the budget off
neotmux.opt.tick_overrun_limit = 5 -- consecutive overruns before the tick is dropped; >= 1
neotmux.resubscribe(name)          -- give a dropped source its tick back; returns handles
```

Logs go to `$XDG_STATE_HOME/neotmux/neotmux.log`, or wherever `--log` says. Notifications also
replay on stderr when neotmux exits, so a message from a frame you never saw is not lost.

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

Fragments from several sources compose in load order, so two plugins both writing to the status
line appear in the order `init.lua` required them.

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

What an overrun looks like from inside your plugin: nothing you can catch usefully. Your code
stops between two instructions with an error you did not raise, wherever it happened to be.
A coroutine does not help — the measurement follows into `coroutine.wrap`. In the log:

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

Nothing is measured and nothing is counted — no hook is armed at all. This is what you want
while stepping through a plugin with a debugger, and it means a `while true do end` freezes
neotmux with no way out but killing it.

### What the budget cannot cut off

The hook that measures you fires between Lua instructions, so it cannot interrupt anything that
is not Lua instructions. It still checks the clock when your tick returns, so these are still
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
- **`pcall`.** The budget error is an ordinary Lua error, so `pcall` catches it. It buys you
  nothing: you still lose the frame and the overrun is still counted against your limit. An
  unbounded `while true do pcall(f) end` does hang neotmux
- **a host binding that blocks.** That would be our bug, not yours

None of this is a sandbox. Plugins are trusted (below); the budget is there so an *accident*
costs one plugin one frame instead of costing you the program.

## Turning a shipped default off

neotmux ships two plugins in the runtime layer an install puts on the search path: `status`,
the default status line, and `prefix`, a `C-a` prefix over `%`, `"` and `o`. Neither is a host
feature — both are ordinary files making ordinary API calls, and the prefix is the only reason
a fresh install can make a second pane at all.

Every layer's `init.lua` runs, earliest layer first, and the shipped layer is last — so your
own `init.lua` can decide what the shipped ones do:

```lua
-- ~/.config/neotmux/init.lua
require('neotmux.defaults').status = false
require('neotmux.defaults').prefix = false
```

That is a Lua module the runtime layer ships, not a host feature: neotmux has no idea the
defaults table exists. Deleting the installed `lua/plugin/` directory works too, and leaves a
multiplexer with no status line, no prefix and no error.

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

## What the sandbox is not

Plugins are trusted. They run in-process with no isolation, and the trust model is "you
installed it". What is removed is removed because it would hang or crash the program, not to
contain a hostile plugin: `io` and `os.execute` block, `dofile`/`loadfile`/bytecode chunks can
crash the interpreter, and `debug` can reach anything. `load` works and gets the real globals.

Do not install a plugin you would not run as a shell script.

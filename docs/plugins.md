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

Until M2 lands there is one pane and `split`/`focus` only log that they arrived. The signatures
will not change.

### Everything else

```lua
neotmux.log.info(text)    -- also .warn, .error
neotmux.notify(level, text)  -- 'info' | 'warn' | 'error'; shown on screen and logged
neotmux.quit()
neotmux.opt.tick_budget_ms = 8     -- per source per frame; 0 turns the budget off
neotmux.opt.tick_overrun_limit = 5 -- consecutive overruns before the tick is dropped
neotmux.resubscribe(name)          -- give a dropped source its tick back
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

## When things break

- **A tick raises.** That source's drawing for the frame is thrown away and its previous frame
  is shown again. Other sources are unaffected. The error is logged with your name.
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
already drew, because your frame is their union. Your previous frame is shown again. Other
sources are untouched, and your keymaps and event handlers keep working.

After **5 consecutive** overruns your tick subscription is removed and a notification names
you. Now nothing refreshes your drawing, so it is cleared rather than frozen. One tick that
finishes inside the budget wipes the count, so this only happens to a plugin that is over
every single frame.

### Getting back

```lua
neotmux.resubscribe('plugin/clock')   -- true if a subscription was restored, false if not
```

The name is the one the notification used. It is a display name, and two runtime layers that
provide the same relative path share one — if that happens, the earlier layer's copy is the one
restored. You get a full 5 frames again, so a plugin that has not been fixed is dropped again.

Re-`require`ing the module instead is also fine and drops what was parked, so you do not end up
subscribed twice.

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
your overruns: you lose the frame, you are named in the log, and it counts towards the five:

- **one long C call.** `string.rep(s, 1e8)`, a huge `table.concat`. Your code is not stopped —
  there is no way to stop it mid-call — so the whole call runs and the rest of neotmux waits
  for it. Measured: a tick doing one 120 MB `string.rep` blocked the frame for 189 ms against a
  5 ms budget. The log says so: `tick exceeded its 5 ms budget: 189 ms in a call that could not
  be cut off`
- **`__gc` finalizers.** Lua switches the hook off while a finalizer runs, so the same applies.
  Measured: 162 ms in one finalizer
- **`pcall`.** The budget error is an ordinary Lua error, so `pcall` catches it. It buys you
  nothing: you still lose the frame and the overrun is still counted against your limit. An
  unbounded `while true do pcall(f) end` does hang neotmux
- **a host binding that blocks.** That would be our bug, not yours

None of this is a sandbox. Plugins are trusted (below); the budget is there so an *accident*
costs one plugin one frame instead of costing you the program.

## Reloading

There is no reload function. There are two lines of Lua:

```lua
package.loaded['plugin.clock'] = nil
require('plugin.clock')
```

Note what this does *not* do: your old registrations are still there. (The exception is a tick
the budget took away: registering a tick again drops what was being held for you.) Keep the handles and
remove them first if re-running the chunk would register twice.

## What the sandbox is not

Plugins are trusted. They run in-process with no isolation, and the trust model is "you
installed it". What is removed is removed because it would hang or crash the program, not to
contain a hostile plugin: `io` and `os.execute` block, `dofile`/`loadfile`/bytecode chunks can
crash the interpreter, and `debug` can reach anything. `load` works and gets the real globals.

Do not install a plugin you would not run as a shell script.

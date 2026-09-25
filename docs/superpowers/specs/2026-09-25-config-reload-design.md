# neotmux — reloading the config without losing the panes

Date: 2026-09-25
Status: proposed
Scope: #38. A `config-reload` command that re-runs every layer's `init.lua` in the running
server, and the API that makes re-running a plugin safe: named groups, modelled on nvim's
augroups.

## 1. Goal and non-goals

### Goal

Lua lives in the server, and `init.lua` runs once, when the server starts. Detach and attach
reset nothing. The only way to pick up an edited plugin is to restart the server, which loses
every pane.

The recipe `docs/plugins.md` gives — clear `package.loaded`, `require` again — re-runs the
chunk but leaves its old registrations in place: every `on_tick`, `on_event`, `on_key`,
`on_write`, `reserve` and `float` is there twice, and `keymap.set` and `command.add` refuse the
plugin its own names, because the holder is its previous self.

After this: edit any file under a layer's `lua/`, run `config-reload`, and the server is
running the new code, with nothing of the old left over — including a plugin whose `require`
line was deleted, which is gone.

### The stance

nvim has no plugin reload either. Re-sourcing is safe there because the API makes the
ordinary way of writing a registration the idempotent way: a keymap set again replaces
itself, and an augroup created again with `clear = true` empties itself. What the API cannot
make idempotent — a timer, a job — is the plugin's to clean up. neotmux takes the same stance
(plugins are trusted, a plugin that is not written to be re-run is the plugin's bug), and so
owes the same thing first: an API where the idempotent way is the easy way.

Today it is not. A plugin that writes `neotmux.on_tick(fn)` the ordinary way doubles on every
re-run, and most of what a plugin holds on the host — a band, a float, a freeze, a write guard —
is held for as long as a handle lives, which a re-run can only leak, not overwrite. So the
groups come first, and the reload is small once they exist.

### Non-goals

- **A fresh VM.** The VM is kept. Throwing it away is simpler to state but has to undo, by
  hand, every effect Lua ever left on the host; a group already knows what it owns.
- **Reloading one plugin.** The unit is the whole config. One plugin can still be reloaded the
  old way by hand, and is safe to be if it uses a group.
- **Reloading on file change.** A command, nothing watching the filesystem.
- **Keeping state across a reload.** A module's locals are new after it; a plugin that wants
  something to survive can write it somewhere in its `unload` hook. No save-and-restore hook.
- **`pane.freeze` as a count (#39).** Separate. A plugin that froze a pane thaws it in
  `unload`.

## 2. The API

```lua
local g = neotmux.group('plugin.clock')   -- the group of that name, emptied

g.on_tick(fn)            g.on_event(name, fn)     g.on_key(fn)      g.on_write(fn)
g.float(rect)            g.reserve(edge, cells)
g.keymap.set(spec, fn)   g.command.add(name, fn, opts)
g.job.start(cmd, opts)   g.timer.every(ms, fn)    g.timer.after(ms, fn)

g.on_event('unload', fn) -- called once, just before a reload
```

**`neotmux.group(name)`** answers the group of that name, emptied of whatever it held. Names
are one namespace across every source, as augroup names are: two plugins that pick the same
name share a group, and either one's re-run empties both. The convention, stated in the docs,
is the module's own name, `'plugin.clock'`. A name that is not a non-empty string raises. A group may be named at any time, not only at
load: one first named from a callback is emptied on a reload that does not name it again,
which is right, since the module that would arm it is new and has not.

**Every `g.*` is the `neotmux.*` of the same name**, with the registration marked as the
group's. Arguments, return values and the two-valued refusals are unchanged, and so is who
owns the registration — the chunk the callback was written in (`docs/plugins.md`, "How your
code is identified"). A group is a second label beside the owner, not a replacement for it:
the tick budget, error attribution and a failed load's sweep all still go by owner.

**Emptying a group** — by `neotmux.group` naming it again, or by a reload that did not name it
(§3) — takes back everything it holds:

| held | taken back how |
|---|---|
| `on_tick`, `on_event`, `on_key`, `on_write` | removed, as `handle:remove()` |
| `float`, `reserve` | removed, as `handle:remove()` |
| `keymap.set`, `command.add` | unbound, unregistered |
| `job.start` | killed, with no `on_exit`, as a failed load's jobs are |
| `timer.every`, `timer.after` | stopped |

The job gets no `on_exit` for the reason a failed load's does not: the callback is the old
code's, and calling into a chunk that is being replaced is worse than not calling.

**Outside a group nothing changes**: `neotmux.on_tick` and the rest register as they do today,
and a chunk re-run with them registers twice.

**Same-source re-registration replaces.** `keymap.set` and `command.add` stop refusing a name
held by the source that is asking: the second registration replaces the first. A different
source still loses, logged and answered `nil, message`, as today. This holds in a group or out
of one, so a plugin re-run by hand without a group keeps its keys and commands.
`command.remove(name)` is added, answering whether there was a command of that name to remove.

**`unload`** is an event like any other, with no fields. It fires once per reload, before
anything is taken back (§3), and is for what no group can see: a pane the plugin froze, a
state it raised somewhere else. Registered in the plugin's group, it is itself replaced on the
re-run rather than stacked.

## 3. The reload

`config-reload` is a command like any other, reachable from the palette, a key, or
`neotmux.command.run`. It is not reachable from `neotmux` on the command line: the CLI runs only
the host's own commands, by design, and a host `reload` command is a possible follow-up. The
command itself is Lua: `neotmux.reload()` asks for the reload, and the
shipped runtime registers `config-reload` over it, in a group of its own. A config that turns
the shipped plugins off still has `neotmux.reload()` to bind.

It cannot reload from where it runs: the command is a Lua callback, and the VM it would be
replacing the code of is the one it is running in. So it only asks. The host marks a reload
pending, and carries it out at the end of the frame — after every session's jobs, timers,
ticks and key dispatch for that frame, before the next one begins — in this order:

1. **`unload`.** Dispatched once, not per session, with the session that asked bound as the
   ambient session. Measured under the tick budget like every other callback.
2. **Empty every group.** Everything every group holds is taken back, and every group is
   marked as not yet named in this reload. The config then runs with no name held through a
   group, as at start, so an earlier layer can claim a key or a command a later layer's group
   held.
3. **Back to a start's settings.** Every `neotmux.opt` is reset to its default and every
   source's overrun count to none, and every tick the budget parked is forgotten: it is in no
   group, and kept, a later `resubscribe` would bring the old code's callback back.
4. **Forget the modules.** Every entry of `package.loaded` that the layer searcher loaded —
   the modules found under a layer's `lua/` — is cleared. The standard library and the host's
   own tables are not.
5. **Run every layer's `init.lua` again**, earliest layer first, under the same load convention
   as at start: a `require` in the init chain that fails is swept, logged and notified, and the
   rest of the config still loads.
6. **Forget the unnamed.** Every group still marked is forgotten. A plugin whose `require` line
   was deleted never named its group this time, so it is gone.
7. **Draw.** The next frame is composed from the new registrations.

The VM, and its globals, are the same ones. A module's locals are new, because the module
ran again: detect's holds, the palette's query, copy mode's cursor, whether the sidebar was
folded. Whatever the host can answer again — the panes, the windows, what a pane is running —
is simply asked for again.

`neotmux.defaults` is a module too, so it is read again: change `sidebar = true` in `init.lua`,
reload, and the sidebar loads.

**Bands keep their order.** A band's place on its edge is the order it was reserved in. On a
reload every group is emptied before the config runs and re-filled in load order, so the bands
come back in the order they went.

**Nothing else runs during a reload.** The server is one thread and the reload is one pass of
its loop, so a key that arrives during it is read after it. This is to be checked against the
serving loop before it is relied on (§6).

## 4. The shipped runtime

Every file under `runtime/lua/` that registers anything with the host names a group at its
top and registers through it. That is not only `plugin/*`: the convention modules that
register at their own top level — `neotmux.detect`, `neotmux.guard`, `neotmux.hints`,
`neotmux.list`, `neotmux.mode`, `neotmux.prompt`, `neotmux.seen`, `neotmux.state` — are
cleared from `package.loaded` and run again with the rest, and would double without one.

`plugin.copy` and `neotmux.hints` freeze panes, and add an `unload` that thaws what they froze.
Nothing else shipped holds anything a group does not.

`examples/` — `clock.lua`, `prefix.lua`, `pane_labels.lua` — are written the same way, since
they are what a plugin author copies.

## 5. Docs

- `docs/plugins.md`, "Reloading": rewritten for `config-reload`, groups, `unload`, and what a
  registration outside a group does on a re-run (it doubles).
- "Registering": `neotmux.group` in the table; `command.add`'s row loses "there is no `remove`";
  the same-source rule next to `keymap.set`'s refusal.
- Lines 346 and 1103, which still say a re-attach re-runs `init.lua`, which stopped being true
  when Lua moved into the server.
- `CHANGELOG.md`: an entry under Unreleased.

## 6. Tests

- **A reload leaves what a start leaves.** A server running the shipped runtime, reloaded,
  has the same bands, floats, key bindings, commands, tick and event registrations, jobs and
  timers as one that has just started. This is the test the design rests on: the one way a
  reload breaks is something left behind.
- **Groups.** Naming a group again empties it; each `g.*` answers what its `neotmux.*` does,
  refusals included; a job in an emptied group is killed with no `on_exit`; a timer in one
  never fires again.
- **The reload.** `unload` runs once per reload; a group not named during a reload is emptied;
  a plugin whose `require` failed does not stop the rest loading; a reload asked for from a
  command happens after the frame, not inside the callback.
- **Same source replaces.** `keymap.set` and `command.add` from the same source replace; from a
  different source they are refused as before. `command.remove` removes.
- **In a real session.** A copy mode left open across a reload leaves no pane frozen; a key
  typed straight after the reload reaches the new code. Checked by running neotmux, not only
  by the suite.

Before any of it is relied on, two facts are read from the code: that the serving loop does
read no key between the end of a frame and the reload, and where in that loop "the end of the
frame" is for every session at once.

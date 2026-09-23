# neotmux — sessions move into the server

Date: 2026-09-23
Status: proposed
Scope: a session stops being a process and becomes a value the server holds. One server, one
socket, many sessions, one Lua layer that can see all of them.

## 1. Goal and non-goals

### Goal

The thing this is for is stated plainly: **neotmux should be able to replace herdr.** herdr is
a daemon that keeps terminal sessions for coding agents, lets an agent drive its own pane
through a socket rather than through simulated keystrokes, and — the part that is actually the
product — shows you which of your agents is working, which is blocked, and which is idle, so
you can go to the one that wants you.

Measured against that, neotmux has two of the four:

| what herdr does | neotmux |
| --- | --- |
| sessions live in a daemon and survive disconnect | yes |
| the daemon watches what a session is doing while nobody is looking | yes, since the Lua layer moved into the server |
| an agent drives its pane through an API rather than through keys | no — `2026-09-23-lua-on-the-server-design.md` §9, not built |
| all the sessions in one view, and a way into any of them | half: they are all in one server and `ls` lists them, but the view and the way in are Lua's, and Lua cannot ask yet |

The fourth is this spec. And the fourth is not "a list of names": a list of names is
`neotmux ls`, which already exists and is not herdr. What makes it herdr is that the row for
each session says what that session is doing, which means whatever draws the row has to be able
to **read a session it is not attached to**. That single requirement is what decides the shape
below.

### Non-goals

- **The command surface.** `neotmux.command(name, fn)`, and the CLI and the agent-facing socket
  API that reach it, are the next spec. They are the third row of the table, and they build on
  this one: switching a session will be one of the commands registered.
- **Command palettes, forms, and floats that take the cursor.** Later, and on top of the
  command surface.
- **Rects and grids composed in Lua.** A separate question, deliberately left open.
- **Several servers.** tmux has `-L`/`-S`; neotmux gets one server per user and a fixed path.
  Adding a second is a flag and a path, and it is not needed to answer anything here.
- **Per-session options.** tmux's options are per session, per window and per server. Every
  `neotmux.opt` field stays server-wide here. `pane_gutter` is the one that will want splitting
  first; it can be split later without changing anything in this spec.
- **Threads.** The server stays one loop. §6 leans on that.

## 2. What changes, in one paragraph

Today a session is a process: one server per session, one socket per session named after it,
one lock beside it, and `neotmux ls` is a directory walk with a connect probe because there is
nothing else to ask. After this change there is one server for a user, on one socket, holding
`BTreeMap<Name, Session>`; `ls` and `kill` are questions asked of it; a client attaches to a
session by name and switches to another with a message rather than a reconnection; and the one
Lua layer, which the last change put in the server, can see every session rather than the one
it happened to be started in. Every existing plugin keeps working unchanged, because the
session a dispatch is for is ambient rather than an argument (§6).

## 3. Why one server and not many

The obvious cheaper design is to keep one process per session and let a client hold several
sockets, switching by re-attaching. It was the recommendation in this conversation until the
goal was stated precisely, and the reason it stops working is §1's: a sidebar that shows each
session's state has to read sessions it is not attached to. With a process per session that is
a new inter-server query layer — a second protocol, on top of the one being simplified, whose
only job is to let one session ask another what it is doing. The cross-session reach has to
exist either way. The question is only whether the thing that has it is one process or many,
and one process is the one that does not need a protocol to have it.

The other argument for many processes was blast radius: a plugin that wedges should cost one
session rather than all of them. That argument is already spent. It was spent on the client
side by `2026-09-23-lua-on-the-server-design.md`, which moved the runtime into the server for
this same goal and accepted the cost, bounding it with the tick budget instead (§4 there).
Nothing here makes that worse. §6 shows why.

## 4. The socket layer

One server for a user, one socket, one lock:

```
$XDG_RUNTIME_DIR/neotmux/server.sock
$XDG_RUNTIME_DIR/neotmux/server.lock
```

with the same `/tmp/neotmux-<uid>/` fallback as today, decided by the same code. What goes is
the pair per session — `<name>.sock`, `<name>.lock` — and with it the reason `ls` reads a
directory.

Everything else in `sock.rs` stays as it is and keeps doing the job it does: `connect` is still
how "is one running" is asked without a race, `Lock::take` is still what makes two clients
starting at the same instant produce one server, `bind`/`unbind` still hand the socket over and
take it away, and `holder` still answers who has the lock. They stop being about a session and
become about the server, which is a smaller thing to be about, not a larger one.

`validate_session_name` stays. A name is still a name: it is a key in a map, it is typed on a
command line, and it is shown in a list.

## 5. The command line

| today | after |
| --- | --- |
| `neotmux [name]` connects to that name's socket, starting a server for it if nothing answers | attaches to that session in the server, **creating it if it does not exist**, starting the server if nothing answers. tmux's `new-session -A -s name`, which is what people mean |
| `neotmux ls` walks the socket directory and probes each one | asks the server. Nothing running is "no sessions", said once |
| `neotmux kill [name]` signals the pid in that name's lock | asks the server to end that session. The server ends when its last session does |

Two things this quietly fixes. `kill` stops being a signal to a process and becomes a request
the session can be ended cleanly by — the signal path existed because there was no protocol to
ask through, and now there is. And `ls` stops being able to lie: a socket file whose server
died answered `ECONNREFUSED` and was filtered out by probing, and a server that answers is the
list.

Killing the server itself — tmux's `kill-server` — is not given a word here. Ending every
session ends it, and `kill` on the last one is that.

## 6. One Lua layer, and the ambient session

There is one VM in the server, not one per session.

This is a conclusion rather than a choice. The server is a single loop: a plugin that fails to
return stops the frame, and the frame is every session's. Splitting the VM per session would
therefore buy no containment at all — the wedge is in the loop, not in the interpreter — while
costing the one thing this spec exists for, which is a plugin that can see across sessions. A
design that pays a real price for nothing is not a trade-off.

What it does cost is that state which was implicitly per-session — implicitly, because a server
*was* a session — has to become so explicitly. In `LuaSubsystem` today, `geometry`, `pane_info`,
`reservations`, `window_ids`, `floats` and the draw `buffers` are one each. `sources`,
`registry`, `opts`, the budget and the overrun record stay server-wide, because a subscription,
a source name and a budget are the plugin's rather than the session's. `ops` gains a session
tag: an op raised during a tick has to be applied to the session whose tick it was.

**Corrected, 2026-09-23.** Of those six, two need it. Building it said so:

- `reservations` and `floats` are already a declaration plus a per-frame placement — `add`,
  `totals` and `lay_out(surface, applied)` — which is the rule §7 states from the other side.
  Nothing to key.
- the draw `buffers` are scratch for the dispatch in hand: `begin`, `record`, `commit` and
  `applicable` all happen inside one session's tick.
- `window_ids` is a counter, and server-wide makes an id name one window for the life of the
  server, which is what a `$1:@3` display would want anyway.

What is left is `geometry` and `pane_info`, and those need no change to the API's closures
either: they capture one `Rc<RefCell<Geometry>>` between them, so that cell is defined as the
current session's view and binding a session swaps the stored one in.

**The session a dispatch is for is ambient.** The host binds it around every tick, every key
and every event, and every existing call — `neotmux.pane.list()`, `window.current()`,
`ctx:text`, `neotmux.reserve` — acts on it. This is the whole reason a plugin written before
this change keeps working: `status.lua`, `sidebar.lua`, `borders.lua` and `copy.lua` never
mention a session, and after this change they still do not, and they now run for each of them.

The alternative — threading a session argument or a session-scoped `ctx` through every call —
was rejected on exactly that ground. It would rewrite every plugin, including the seven shipped
ones, to say something the host already knows.

## 7. A handle is a declaration; the instance is the session's

`sidebar.lua` opens with, at file scope:

```lua
M.band = neotmux.reserve("left", M.widths.open)
```

That runs once, during load, before any session exists — and it has to keep meaning what it
means, which is "this plugin wants a band on the left of whatever it is drawing into". The
float in `float_probe.lua` is the same shape held in a module-level variable, made once and
reused every tick.

So: **a handle names a declaration, and each session holds an instance of it.** A reservation
declared once is placed in every session, and `ctx:reserved()` answers where it landed in *this*
session. A float created once is a float in every session, and `ctx:float_rect(win)` answers
for this one. `handle:resize_to(n)` changes the declaration, so the fold in `sidebar.lua`
folds the sidebar everywhere, which is what a person who folded it would expect.

This rule is what makes §6's ambient session complete. Without it, a plugin that made a float
during session A's tick and reused the handle during session B's would be holding something
that exists in one place and is drawn in neither, and every plugin author would have to learn
that. With it, there is nothing to learn.

Window ids are the exception that proves it: they name a window inside one session's tree, so
they are per session and are not declarations. `window.create()` is a call made during a tick,
and the tick is a session's.

## 8. Reading another session

Everything in §6 and §7 is about the session a dispatch is for. The sidebar needs the others.

```lua
for _, name in ipairs(neotmux.session.list()) do
  neotmux.session.with(name, function()
    -- pane.list, window.list, pane.info, pane.content: all of this session
  end)
end
```

`with` rebinds the ambient session for the duration of the call and puts it back afterwards,
including when the function errors. That is the whole mechanism: one function, and every
existing read works inside it, including the pane reads a watcher is built out of. Nothing new
has to be written per API.

What it deliberately does not give is *drawing* into another session. `ctx` belongs to the
session being drawn, and a `ctx:text` inside a `with` still draws where the tick is. Drawing
one session's panes inside another's surface is the question §1 lists as a non-goal, and it is
the one that would need rects in Lua.

The rest of the surface:

```lua
neotmux.session.current()          -- the name this dispatch is for
neotmux.session.list()             -- every session, in name order
neotmux.session.create(name)       -- an op; the session exists at the next frame
neotmux.session.switch(name)       -- an op; moves the client this key came from
neotmux.session.kill(name)         -- an op
```

`switch` names the one thing a plugin cannot infer: *which* client moves. A key arrives from a
client, so during a key handler there is a client in scope and it is that one. Called from a
tick, where there is no client in scope, it moves every client attached to the current session.
Both of those are stated rather than discovered, and the second is what a session-picker
launched from a tick would want anyway.

`neotmux.session_name()` becomes an alias for `session.current()`. It answered "the session
this client was started with" — a fact that stops existing — and the nearest true thing is the
one `current()` gives.

## 9. The frame with N sessions

One frame, in this order:

1. The door, once, as now.
2. Every session, in name order: drain its panes, tick it, run the Lua layer against it with the
   ambient session bound, apply what that raised. **Every session, attached or not** — a session
   nobody is looking at running its plugins is the thing the last change bought, and this keeps
   it.
3. Every attached client is sent a picture of the session it is attached to. Sessions nobody is
   attached to compose nothing: a picture is made for a client, and there is no client.
4. The rest of the frame's time to the clients and the panes, as now.

Step 2 is O(sessions) of Lua work per frame where it used to be O(1), and the budget is per
source per dispatch, so ten sessions with a busy status line is ten status lines' worth of work
in one 16 ms frame. That is a real bound and it is not addressed here. What makes it tolerable
to defer: the number is small, it is the same work ten separate servers did, and it is now
visible in one place rather than spread across ten processes where nobody could see it.

Step 3 is where the per-client work that `Attached` lost in the last change comes back, one
field's worth: which session this client is looking at.

## 10. Attaching, and switching

```
client → server :  Attach { size, session }, Key, Resize
server → client :  Frame, End, Clip, Bye
```

`Attach` gains the session's name, and that is the only change to the wire. There is no
`Switch` message, because switching is not something a client decides: a key goes to the
server, a plugin decides what it meant, and `neotmux.session.switch` is an op like every other
(§8). The server moves the client and the next frame it sends is the new session's. The
connection is not touched, so a switch costs a frame rather than a reconnection, a handshake
and a fresh Lua load — which is the concrete thing the one-process-per-session design could not
have given.

`PROTOCOL_VERSION` goes up. No compatibility path, for the reason it never has one: the client
and the server are the same binary.

A client is attached to exactly one session at a time. Two clients on one session is unchanged
from `2026-09-23-lua-on-the-server-design.md` §8; two clients on two sessions is now possible
and is the same code — a picture is composed per client out of the session that client names.

## 11. Staging

1. **One socket, one server, one session in it.** The socket layer, the lock, the CLI, and a
   `Sessions` map with exactly one entry. Nothing about the Lua layer moves. At the end of this
   the behaviour is identical and the shape is the new one.
2. **The ambient session.** The per-session state in `LuaSubsystem` is keyed, the host binds
   the current session around each dispatch, and handles become declarations with per-session
   instances (§7). Still one session, so still no behaviour change — and this is the stage that
   is nearly all of the work.
3. **N sessions.** The frame loops over them, `Attach` names one, `Switch` moves a client, and
   `ls`/`kill`/`create` reach the map.
4. **`neotmux.session.*`**, including `with`. The sidebar grows a session section, in Lua, and
   the goal in §1 is met to the extent that anything without the command surface can meet it.

Stage 2 is the one to be careful with, and the one to plan on its own. Stages 1 and 3 are
mostly moving a boundary that is already drawn.

**Reordered, 2026-09-23.** Stage 1 as written puts the socket first, and it cannot go first:
`crates/neotmux/tests/sessions.rs` runs `work` and `default` at the same time and pins that
`ls` lists both and that `kill work` leaves `default` alone. One socket over a map that can
hold one entry breaks that, and it would stay broken until the whole of stage 2 had landed.
So the internals go first and the socket goes with N sessions:

1. **The word, with one session.** `SessionId`, the map, the ambient binding, ops tagged.
   Nothing observable changes. Done, in `plans/2026-09-23-the-word-session.md`.
2. **The socket, the CLI and N sessions**, in one step, so the two-name behaviour moves from
   two servers to two sessions in one server rather than being absent in between. Done, in
   `plans/2026-09-24-one-server-many-sessions.md`. `Attach` carries the name the client asked
   for; the wire gained `List` and `Kill` up and `Sessions` and `Killed` down, which §6's
   table does not list because they are asked at the door rather than by a client;
   `neotmux.session_name()` answers the session whose dispatch is running, and `nil` during a
   load.
3. **`neotmux.session.*`**, including `with`, and the sidebar that uses it.

## 12. Tests

- Two sessions exist at once in one server, and a key typed in one does not reach the other.
- A client switches from one to the other and back, on one connection, and sees each one's
  panes.
- A plugin's `on_tick` runs for every session, including one nobody is attached to — the
  detached-tick test from the last change, with a second session standing beside it.
- A reservation declared once is placed in every session, and folding it folds it in both (§7).
- A float made once is drawn in every session (§7).
- `neotmux.session.with` reads another session's panes, and a `ctx:text` inside it still draws
  in the session the tick is for.
- `ls` with no server running says so; `ls` with two sessions lists both; `kill` on one leaves
  the other; `kill` on the last ends the server.
- The existing end-to-end suites pass unchanged, which is the claim in §6 that a plugin written
  before this does not know it happened.

## 13. What this does not touch

The pane tree, the layout, the terminal emulation, the scrollback and the frame's composition —
all unchanged, all still the session's. The door's non-blocking accept, the greeting and the
grace poll are about connections rather than about what is behind them and are untouched. The
crate boundaries in `2026-09-23-crate-boundaries-design.md` hold. The tick budget and its seal
are untouched, and §6 leans on them being there.

## 14. Corrections to earlier specs

- `2026-09-21-sessions-design.md` describes one socket per session, a lock beside each, `ls` as
  a directory walk with a connect probe, and `kill` as a signal to the pid in a lock, with the
  observation that neither needs a protocol. That was true of a server that was a session. §4
  and §5 here replace all four; the reasoning in that spec about what a socket can be asked
  without a race is still the reasoning `sock.rs` runs on.
- `2026-09-23-lua-on-the-server-design.md` §7 says "one VM per server, which is to say one per
  session". Those were the same sentence and are now two: §6 here keeps the first and drops the
  second.
- The same spec's §12 lists "the socket layer, the lock, the directory, one file per session"
  among what it does not touch. It did not. This spec does, and §4 is the whole of it.

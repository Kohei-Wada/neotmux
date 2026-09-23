# neotmux — the Lua layer moves to the server

Date: 2026-09-23
Status: proposed
Scope: reversing the M3 decision that put the Lua layer on the client. The plugin runtime
becomes the session's, running in the server process beside the panes it talks about, and the
client becomes what nvim's remote UIs are: keys up, cells down.

## 1. Goal and non-goals

### Goal

The thing neotmux is for is customisation: a terminal multiplexer whose behaviour is a Lua
program rather than a fixed feature list. The measure of that goal is what a plugin can be
made to do, and today a plugin cannot be made to do the most interesting thing a multiplexer
could do for its user — watch a long-running job while nobody is looking at it. A plugin
cannot do it because the plugin is not running. `session.rs` says so in as many words:

> Nobody is subscribed to anything any more, because nobody is running any plugins.

A detached neotmux server drains its panes into their terminal state and does nothing else.
Everything a person might want automated — noticing that a coding agent has stopped and is
waiting to be answered, answering it, starting the next job when this one finishes, restarting
something that died, anything on a timer — happens in the window where no plugin exists.

After this change the Lua layer runs in the server. It ticks whether or not a terminal is
attached, it reaches the panes by calling a function rather than by asking across a socket,
and the same plugin API that draws a status line can watch a pane at three in the morning.

### Non-goals

- **The command surface.** `neotmux move-pane 1 2` — a CLI that reaches a running session and
  asks it to do something — is the natural next thing this change makes possible, and §9
  writes down its shape so the design does not foreclose it. Building it is not in this spec.
- **A second Lua VM.** One VM per session, not one per client and not a restricted second one
  beside the first. §7 says why.
- **Per-client surfaces.** A session has one surface, as it does today and as tmux does. §8.
- **`window-size` beyond `latest`.** tmux offers `largest`, `smallest`, `manual` and `latest`.
  neotmux implements `latest`, which is tmux's default and is what the code already does.
  The other three are an option to add later, not part of this move.
- **Sandboxing the plugin from the session.** A plugin that runs in the server can wedge the
  session. §4 bounds that as far as it can be bounded; it does not eliminate it, and §4 is
  honest about which hole stays open.

## 2. What changes, in one paragraph

The `LuaSubsystem` is built in the server process instead of in `client()`, and the frame
stops being a conversation. Today a frame is four messages — the server's `Tick`, the client's
`Tock`, the server's `Done`, and a `Read`/`Reading` pair for every question the tick asked —
because the code that decides what the frame contains lives on the far side of a socket from
the state it decides about. Once both are in one process those four messages become function
calls and leave the wire entirely. What is left on the wire is what a remote UI needs: the
client sends `Attach`, `Key` and `Resize`; the server sends a composed `Frame` and, at the
end, an `End`. The frame composition and the frame's wire format already exist and do not
change — `Done.frame` is already a server-side composition of panes, overlay and notices that
the client only paints.

## 3. Why the line moves

M3 put Lua on the client deliberately, and the reason is recorded in the roadmap:

> **Lua stays on the client**: the server holds other people's long-running jobs, so a frozen
> client should cost one client, not every session.

That reason was sound and is still the one real cost of this change. Two things have happened
since.

The first is that the goal has been stated more precisely. "Customisable" was the word, but
the target is now a plugin API on which a session-watching agent runtime could be written by a
user rather than shipped by us. That target is unreachable with the runtime on the client, at
any level of API quality, because the runtime is not running when the work needs doing. The
M3 line does not trade some customisation for safety; it removes a whole category of plugin.

The second is that the containment the M3 reason was worried about can be built. §4.

A note on the analogy, because two different ones were in play while this was decided. The
browser's model — a dumb server, the interesting code in the client, and a page whose script
dies when the tab closes — describes M3's neotmux exactly, including the limitation. Neovim's
model is the opposite: `nvim --embed` holds the Lua, the buffers and the plugins, and a remote
UI is a thin client that sends keys and receives a grid. neotmux's server already sits where
nvim's core sits, holding the panes and composing the frame; it just has no brain. This spec
puts the brain where the state is.

## 4. The budget, made real

A plugin in the server can hang the session, so the interpreter must be interruptible. Today's
budget (`crates/neotmux-lua/src/budget.rs`) samples the clock from a count hook every 1000
instructions and raises a Lua error once the deadline has passed, and `crate::overrun` parks a
source that overruns repeatedly. It already stops the obvious cases, with tests: a bare
`while true do end` is cut off, and `set_global_hook` rather than `set_hook` keeps a coroutine
from escaping. Three holes remain, and this section is a prerequisite for §5, not a follow-up
to it.

**`pcall` swallows the interrupt.** The hook raises an ordinary Lua error, so a loop that calls
anything inside `pcall` catches the budget's own error and goes round again. Measured against
a 5 ms budget, `while true do ... pcall(f) ... end` ran 892 ms without being interrupted and
without being recorded as `Interrupted`; with the loop's own escape hatch removed it does not
return at all. On the client that wedges one client, which is what M3 chose. In the server it
wedges the session for ever, which nothing recovers from.

The fix is the one `vm.rs` already uses for `load`: seal the global behind a wrapper the plugin
cannot get underneath. `pcall`, `xpcall` and `coroutine.resume` are captured as locals in a
`do` block and replaced with versions that, when the call failed *and* the budget has already
fired, re-raise instead of reporting. The budget's `overran` flag is the one fact the wrapper
needs, exposed as a host function. Measured with that wrapper in place, the same uninterruptible
loop — with no escape hatch of its own — ended at 5.03 ms with `Outcome::Interrupted`, and so
did the same loop hidden inside `coroutine.wrap`. Whether the wrapper is written in Lua or as
host functions is an implementation choice; the Lua form costs a `table.pack` per `pcall` and
the host form does not, so the host form is preferred unless it proves awkward.

**Only ticks are measured.** `Budget::enter`/`leave` wrap tick dispatch and nothing else, which
the module says outright: "key handling, event dispatch and config load are not measured
against any budget." An infinite loop in a key handler or in `init.lua` hangs neotmux today,
before any of this move. The measured region extends to key dispatch, event dispatch and
config load. Config load is the one that needs its own deadline rather than the tick's: a
plugin's `init.lua` legitimately does more work than a frame's worth, and being cut off at
8 ms would make plugins unloadable. It gets a separate, much larger budget.

**A long host call cannot be interrupted.** `Outcome::RanLong` is exactly this: no Lua
instructions run during a host call, so no sample lands, and the tick is attributable but not
interruptible. This one is not fixed; it is designed around. No function on the server-side
Lua API blocks. Anything that would wait — and the interesting ones are all of this shape,
from a timer to a future filesystem or network reach — is expressed as a subscription that
delivers on a later tick, never as a call that returns when the waiting is over. That rule is
part of the API's contract and belongs in the API's documentation, not only here.

`tick_budget_ms = 0` continues to switch the budget off entirely. It exists so somebody can
step through a plugin under a debugger, and the alternative — every tick overrunning at its
first sample and unsubscribing the world — is worse. It is an opt-out with a stated purpose,
not a hole.

This section is independently shippable and worth shipping on its own: the second hole is a
bug in neotmux as it stands today.

## 5. Where everything lives afterwards

The client keeps exactly what needs a terminal: raw mode, reading bytes, decoding them into
keys, painting a composed frame, and noticing `SIGWINCH`. It holds no Lua, no plugin state, no
session state, and it makes no decisions — a key it reads is sent, not judged.

The server gains the `LuaSubsystem`, the plugin registry, the op buffer, key dispatch, the
draw commands and the notices. It already holds the pane tree, the windows, the PTYs, the
terminal state and the frame composition.

Three things stop crossing the socket, and each is a whole module's worth of machinery that
exists only because of the crossing:

- **`reads.rs` and `Read`/`Reading`.** Every question the Lua layer asks about a pane's text is
  a round trip today, and `Freeze`/`Unfreeze`/`IsFrozen` exist so that the answer is consistent
  across a round trip that the pane could move underneath. Beside the panes, these are calls on
  `PaneRead`, and the freezing has nothing left to protect.
- **`Tick`/`Tock`/`Step`/`Event` and the revisions and pane info they carry.** These ship the
  session's state to a Lua layer that cannot see it. The Lua layer can see it.
- **`SessionOp` on the wire.** A plugin's request to split a pane or create a window is a
  message today because the tree is elsewhere. `SessionOp` survives as a type — it is still
  how an op is buffered and how a refusal is attributed to the source that asked — but it stops
  being something that is encoded.

`Attach` shrinks. `first_window`, `reserved`, `gutter` and `wants_pane_output` are all facts a
client currently carries because its Lua layer is the only place they are known; the server
knows all four. What is left is the terminal's size.

## 6. The protocol that is left

```
client → server :  Attach { size }, Key, Resize
server → client :  Frame, End
```

`Frame` and its wire format are unchanged: the server already composes panes, overlay, notices
and rects into one, and the client already does nothing with it but paint. `Refused` stays, for
a version mismatch and for a connection the server turns away.

`PROTOCOL_VERSION` goes up. There is no compatibility path and none is wanted: the client and
the server are the same binary, and the handshake already names both numbers when they differ.

**Amended, 2026-09-23 (stage 2).** The table above is short by two, and both are side effects
that need a terminal:

```
server → client :  Frame, End, Clip, Bye
```

`Clip` carries text a plugin yanked, because `neotmux.clipboard.set` writes OSC 52 to a
terminal and the code that calls it now runs in the process that has none. It goes to every
attached client; a yank with nobody attached is refused, with that as the reason. `Bye` carries
the notifications raised while one client was attached, for it to replay on stderr once its
terminal is restored — `docs/plugins.md` promises that replay, and the `Notifications` that
hold the transcript moved into the process whose stderr is `/dev/null`.

One more thing the move settled: the client attaches and reads the session's first word
**before** it enters the alternate screen. A plugin that calls `neotmux.quit()` while loading
used to be the client's own discovery, made before it touched the screen; the client loads
nothing now, so it asks instead. A session that was over before it began is an `End` and not a
`Refused`, so that client exits cleanly and the terminal is never switched.

The round disappears. Today the client may not touch its keyboard between sending a `Tock` and
reading the `Done`, because the ops it just sent have been applied at the far end and a key
handler running in that window would see a session that had already moved on. With the handler
and the session in one process there is no window and no rule.

## 7. One VM for the session

There is one Lua VM per server, which is to say one per session.

**Amended, 2026-09-23.** Those were one sentence because a server was a session. They are two
now: `2026-09-23-sessions-in-the-server-design.md` puts many sessions in one server and keeps
the first half — one VM per server — while dropping the second. The reasoning below is about
plugin state not being split per *client*, and it survives the change untouched. Plugin state is the session's:
its modes, its registers, its floats, its reservations, its window id counter.

This is the tmux arrangement, and today's is not. tmux's copy mode is a property of the pane —
`pane_in_mode` is "Number of modes pane is in" and `pane_mode` is "Name of pane mode, if any",
both pane formats, with nothing of the kind on a client — so every client looking at a pane in
copy mode sees copy mode. The claim in `2026-09-21-sessions-design.md` §6 that "A client's mode
is its own ... That is tmux's arrangement" is wrong about tmux, and that spec is corrected in
§13. Sharing plugin state is not a concession this move extracts; it is the behaviour neotmux
was supposed to have.

It also deletes a class of problem rather than creating one. Because Lua state is per client
today, two clients have to be reconciled: `wants_pane_output` is an OR across the attached
clients, a reattaching client must not mint a window id that already exists, and the last
client to attach imposes its bands on everyone. Those rules exist to paper over state that was
split. With one VM there is nothing to reconcile, and the tests that pin those rules go with
them (§11).

A per-client fact is still a per-client fact — which terminal a key arrived from, how big it
is — and those belong to the client record in the server, not to a second VM. If a plugin
should ever need to distinguish them, it is an argument to a handler, not a separate runtime.

## 8. Two clients, the tmux way

Two clients on one session stays, because tmux does it and neotmux is neotmux.

A session has one surface. Two clients of different sizes do not get two renderings: tmux has
no such concept either, offering instead one size chosen from the attached clients by the
`window-size` option. neotmux implements that option's `latest` — the size of the client whose
resize arrived most recently — which is both tmux's default and what the code does now. A
client whose terminal is smaller than the surface sees the existing too-small-terminal
behaviour.

What changes for a second client is only that its plugin state is no longer its own, which §7
argues is the correction and not the cost.

## 9. The command surface (shape only, not built here)

The reason to write this down now is that it is the payoff, and a design that made it awkward
would be the wrong design.

A CLI subcommand — `neotmux move-pane 1 2` — connects to the named session's socket and asks
the server to run a command. With the Lua layer in the server, the command it names can be one
a plugin defined: a plugin registers a command the way nvim's `:command` does, and the CLI is
one more way to reach it, beside a key binding. That is the whole point of moving the runtime,
stated as a feature: the set of things a user can drive neotmux to do from outside is the set
of things their plugins define, not a list we ship.

Nothing in §6 forecloses it. A command channel is a third message shape on the same socket, and
a connection that sends a command is not an `Attach`, which the doorstep already distinguishes.

## 10. Staging

1. **The budget, made real** (§4). Independent of everything else, shippable on its own, and a
   bug fix for the code as it stands. Nothing after this starts until it lands.
2. **The VM moves** (§5). The large one. `LuaSubsystem` is built in the server, the tick and
   key dispatch move with it, and the client's frame loop becomes read-a-frame-and-paint. This
   task needs its own decomposition when it is planned; it does not decompose at this level,
   because there is no useful intermediate state between the runtime being on one side and
   being on the other.

   **Done, 2026-09-23**, planned in `plans/2026-09-23-the-vm-moves-to-the-server.md`. The
   `Attach` shrink and the `PROTOCOL_VERSION` bump came with it rather than with stage 3: a
   client with no Lua layer cannot fill `first_window`, `reserved`, `gutter` or
   `wants_pane_output`, so the shrink is forced by the move. What is left for stage 3 is the
   dead vocabulary, which is what that stage was always mostly about.
3. **The sweep** (§5, §6). The wire types nothing references any more come out, `reads.rs`
   goes, `Attach` shrinks, `PROTOCOL_VERSION` goes up.
4. **Two clients, corrected** (§7, §8). After 3 the state is already shared, so this is mostly
   removing reconciliation rules and the tests that pin them, and correcting the sessions spec.

## 11. Tests

What the budget work adds, all in `neotmux-lua`: a sealed `pcall` cannot swallow the interrupt;
neither can `xpcall`, `coroutine.resume`, or a `pcall` inside `coroutine.wrap`; a key handler
that loops for ever is cut off; an `init.lua` that loops for ever is cut off against the config
budget rather than the tick budget; a slow but finite `init.lua` still loads.

What the move preserves: every existing behavioural test over the socket in `tests/socket.rs`
and `tests/fixtures.rs` that is about the session rather than about the split — a keystroke
reaching a shell and coming back, a detach keeping the session, a reattach rebuilding a float,
a session refitted to the terminal that attaches to it, a malformed message costing one
connection and not the session.

What the move deletes, because the thing it pinned is gone:

- `wants_is_the_or_of_the_clients_and_not_the_last_to_say_so`
- `a_resized_handler_on_reattach_does_not_mint_a_window_id_that_already_exists`
- `a_window_created_before_a_detach_and_one_created_after_do_not_share_an_id`

All three pin reconciliation between per-client Lua states. A test is not deleted quietly: each
one's removal is justified by the state it was reconciling no longer being split.

What the move needs that is new: a plugin that ticks while no client is attached does tick, and
what it did is visible on reattach. This is the goal of the whole change and it has no test
today because it has never been possible.

## 12. What this does not touch

The socket layer, the lock, the directory, one file per session, and the handshake's shape.

**Superseded, 2026-09-23.** `2026-09-23-sessions-in-the-server-design.md` touches exactly the
first four: one server on one socket, holding many sessions. It was true that this spec did not
touch them. The
door — the non-blocking accept, the half-finished greeting, the grace poll — is untouched: it
is about connections, not about what runs behind them. Pane management, layout, the pane tree,
the terminal emulation and the scrollback are all already the server's and stay where they are.
The crate boundaries settled in `2026-09-23-crate-boundaries-design.md` hold: what moves is
which process builds `neotmux-lua`, not which crate owns what. `neotmux-lua` keeps
`neotmux-proto` as its only workspace dependency, beside `anyhow` and `mlua`.

## 13. Corrections to earlier specs

- `2026-09-21-sessions-design.md` §6 states that a client's mode being its own "is tmux's
  arrangement". It is not: tmux's modes are pane state (`pane_in_mode`, `pane_mode`), shared by
  every client looking at the pane. The sentence is corrected, and the behaviour it describes is
  changed by §7.
- `2026-09-19-neotmux-design.md` §11 records the M3 decision that Lua stays on the client, with
  its reason. The decision is reversed here. The reason is not dismissed — it is the cost this
  spec accepts, bounded by §4 — and the roadmap entry is amended to point here rather than
  rewritten, so that what was decided and why it changed both stay readable.

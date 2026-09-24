# neotmux — the command surface

Date: 2026-09-23
Status: proposed
Scope: a named thing the server can be asked to do, from the command line and from a key, with
a line drawn between the commands the host owns and the commands a plugin defines.

## 1. Goal and non-goals

### Goal

The third row of the herdr table in `2026-09-23-sessions-in-the-server-design.md` §1: **an
agent drives a session through an API rather than through simulated keystrokes.** Everything
else in that table is built. This is the row that is not, and it is the one that makes neotmux
usable by something that is not a person at a terminal:

```
neotmux new agent-7
neotmux send %4 'cargo test' --enter
neotmux capture %4 --lines 40
```

The second goal is the one the Lua layer was moved into the server for: **the set of things a
user can drive neotmux to do from outside is the set of things their plugins define, not a list
we ship.** A plugin registers a command the way nvim's `:command` does, and the CLI is one more
way to reach it, beside a key binding.

A command palette and a form that collects a value are not features in this spec. They are what
becomes writable once it exists: a palette is a plugin that lists the registered commands and
calls one, and a form is a plugin that collects values and calls one with them.

### Non-goals

- **Asynchronous or long-running commands.** A command returns inside the tick budget or it
  fails. A command that wants to wait for something waits in a plugin's own tick, not in the
  connection.
- **Structured output.** Lines of text and an exit status, the way tmux answers. A `--json`
  flag is a later question, and answering it later costs nothing: adding a flag to a command
  that already prints lines is not a redesign.
- **Interactive commands.** The connection that asked has no terminal and is not a client. A
  command that needs to ask something is a plugin with a float, reached by a key.
- **The pane and window verbs the Lua API is still missing** — `pane.kill`, zoom, swap,
  break/join. They are ordinary additions to `neotmux.pane`, and this spec neither needs nor
  blocks them.

## 2. The line

The question this spec exists to settle is which commands the host implements in Rust and which
are Lua's.

**The host owns the control of the server.** Sessions, the panes in them, writing into a pane
and reading what is on it. These work when the Lua layer is not there at all — when
`runtime/lua/plugin/` has been deleted, when a user's `init.lua` raises on its first line, when
a plugin has been parked for overrunning. A control CLI for a daemon that stops working because
of how the daemon's config is written is not a control CLI.

**Lua owns the vocabulary of the interface.** Splitting a pane, opening a picker, zooming,
copy mode, the sidebar, the status bar. None of it is lowered into the host. This is the
boundary the last three specs have been moving *towards*, and nothing here moves back across
it: `neotmux split-right` is not a host command, it is a command a shipped plugin registers.

The line is not "what is useful", it is "what must survive Lua being broken". That is a small
set, and it stays small.

## 3. What the host commands are

Two of these exist today; the rest are new. Every one of them is answered without the Lua layer
being consulted, entered or woken.

```
neotmux ls                            the running sessions, one per line            (exists)
neotmux kill [name]                   end a session and every shell in it           (exists)
neotmux new [name]                    start a session with no client attached
neotmux rename <old> <new>            rename a session
neotmux switch <name>                 move the attached clients to that session
neotmux windows [-t name]             @1 <name> panes=2
neotmux panes [-t name]               %1 @1 80x24 bash /home/kohei
neotmux send <pane> <text> [--enter]  write text into that pane's pty
neotmux capture <pane> [--lines N]    the pane's visible text, oldest line first
```

`-t` names the session, defaulting the same way an attach does — the name in `-t`, or
`DEFAULT_SESSION` where none is given. `send` and `capture` take a pane id and no `-t`: pane
ids are allocated by one `PaneIds` counter for the whole server, so `%4` names one pane in one
session and nothing else.

`new` is the one with a wrinkle worth stating. A session's surface size comes from the client
that attached, and this one has no client, so it is laid out at 80x24 and refitted when a
client does attach — which is the `window-size latest` rule sessions already follow. An agent
that never attaches a client works at 80x24, which is the size a terminal is when nobody has
said otherwise.

`switch` moves every client attached to this server to the named session. One person at one
terminal usually has one client, and "bring me to that session" is what the command means.

Output is lines on stdout. Failure is a non-zero exit and a sentence on stderr. A command that
names something that is not there — a session, a pane — fails rather than printing nothing:
`neotmux capture %9` on a pane that has gone is a mistake worth an exit status, not an empty
answer.

## 4. What a plugin command is

```lua
neotmux.command.add('scratch', function(args)
  local window = neotmux.window.create()
  neotmux.window.focus(window)
  return 'opened ' .. window
end, { desc = 'open a scratch window' })
```

It is `command.add`, not a callable `neotmux.command(...)`: the top-level tables in this ABI
are sealed, and a callable table is a metatable the seal would have to make an exception for.
`neotmux.keymap.set` is the shape this ABI already uses for "register a named thing", and this
follows it — `desc` is optional, and is what `neotmux.command.list()` reports back for it.

- The name is a word. Registering a name that is already registered is refused, logged, and
  answered with `nil, message` — the rule `neotmux.keymap.set` already follows, and it gives
  the user the name rather than the plugin: a config layer's `init.lua` runs before the
  shipped one.
- `args` is an array of strings, exactly as the command line gave them, with no parsing done
  for the handler. A command that wants flags parses them itself.
- The return is `nil`, a string, or an array of strings, and those are the lines printed. Any
  other type is an error against the plugin, reported the way a tick's error is.
- `error()` inside the handler is a failed command: the message goes to stderr and the exit
  status is non-zero.
- The handler runs with the session it was addressed to bound as the ambient session, so
  `neotmux.window.create()` inside it means that session's, not whichever session ticked last.

A registered command is reachable three ways, and this is the whole point of it being a
registration rather than a key binding: the CLI, a key binding that calls it by name, and
another plugin that calls it by name. The last of those is what a palette is.

## 5. Name resolution

One flat namespace, resolved **Lua first**. `neotmux scratch` runs the plugin's `scratch`;
`neotmux ls` runs the host's `ls` unless a plugin registered `ls`, in which case it runs that.
A plugin can replace what a built-in name does, which is the nvim behaviour and the reason a
user has a runtime layer at all.

The resolution itself never runs Lua. `neotmux.command` writes the name into a table the host
holds in Rust, so "is this name registered" is a lookup on a `BTreeMap`, answerable while the
VM is mid-tick, parked, or in whatever state a raising `init.lua` left it.

The one exception is written in `2026-09-24-writes-pass-a-hook-design.md`: a write to a pane
that a registered hook did not pass is refused, and a hook that fails is a hook that did not
pass it. A guardrail that fails open is useless exactly when something is wrong.

**When Lua fails, the built-in answers.** If the handler raises, overruns its budget, or the
frame does not answer within the connection's patience, and the name is also a host command,
the host command runs and the connection is told what it did. If there is no built-in by that
name, the failure is the answer.

This is a name meaning two things at two moments, which is a real cost and is stated here so it
is not discovered later. It buys the property the whole line in §2 exists for: `neotmux kill`
ends a session even when the plugin that shadowed `kill` is the reason you are trying to end
it.

## 6. Where a command runs

A connection that sends a command is not an `Attach`. The doorstep already distinguishes the
two — `List` and `Kill` are answered there today and never become clients — and a command is
a third shape on the same socket.

**A host command is answered at the door.** It touches the sessions, the panes and the ptys,
none of which are Lua's, and the answer goes back on the connection that asked. No frame is
entered and no tick is dispatched.

**A plugin command is queued for the frame.** The VM is single-threaded and ambient-bound; a
command must run with its session bound, which is something only the frame can arrange. So the
door records the connection as waiting, the next frame binds the session, runs the handler,
and the answer is written to the waiting connection before that frame ends. The connection
waits as long as the doorstep's patience allows, and past that is §5's fallback.

## 7. The protocol

`PROTOCOL_VERSION` goes to 9, and the two messages are:

```rust
ToServer::Command {
    /// The session the command is addressed to. `None` is the default name.
    session: Option<String>,
    name: String,
    args: Vec<String>,
}

ToClient::Ran {
    ok: bool,
    /// stdout where `ok`, stderr where not. Never both.
    lines: Vec<String>,
}
```

`List` and `Kill` stay as they are rather than becoming `Command`s. They are answered by the
same code either way, and rewriting a working door handshake to save two discriminants is
churn. What is worth saying is that they are the precedent this is built on, not a duplicate
of it.

## 8. Testing

- **The host commands, against a live server**, in the socket tests that already start one:
  `new` then `ls` names it; `send` then `capture` shows what the shell printed; `panes` names
  the pane `send` wrote to. These are the tests that say the agent-facing path works.
- **Name resolution, at the Lua layer**: a plugin registering `ls` shadows the built-in; a
  plugin registering nothing leaves it alone; a handler that raises falls back to the built-in
  where one exists and fails where one does not.
- **The ambient binding**: a command addressed to a session creates its window in that session
  and not in another that ticked more recently. This is the bug §4's last bullet exists to
  prevent, and the Lua-layer harness already builds two sessions.
- **The wire**: `Command` and `Ran` round-trip, in the codec's existing fixture style.
- **The refusals**: a command name nobody registered, a pane id that is not there, a session
  name that is not running — each a non-zero exit with a sentence, asserted as text.

## 9. Staging

1. **The wire and the door.** `Command`/`Ran`, the doorstep arm that reads one, and the two
   host commands that need nothing new underneath: `new` and `rename`. Shippable: `neotmux new
   agent-7` works at the end of it.

   **Done, 2026-09-24**, planned in `plans/2026-09-23-the-host-commands.md`.
2. **The reading and writing commands.** `panes`, `windows`, `send`, `capture`. This is the
   row of the herdr table, and at the end of it an agent can drive a session it started.

   **Done, 2026-09-24**, planned in `plans/2026-09-23-the-host-commands.md`.
3. **`switch`.** Small, and separate because it is the one host command that moves a client
   rather than touching a session.

   **Done, 2026-09-24**, planned in `plans/2026-09-23-the-host-commands.md`. `switch_clients`
   is lifted out of `ServerOp::Switch`'s arm in `apply_server_ops` so the plugin path and the
   command path share one implementation: a plugin moves the clients of the session that
   asked, and the command moves every client attached to this server.
4. **`neotmux.command` and Lua dispatch.** The registration table, the frame's queue, name
   resolution and the fallback. Nothing before this depends on it, and everything a plugin
   wants from this spec arrives here.

Each stage leaves the tree green and the CLI honest about what it has.

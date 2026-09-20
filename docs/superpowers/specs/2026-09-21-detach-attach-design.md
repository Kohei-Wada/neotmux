# neotmux — detach and attach

Date: 2026-09-21
Status: proposed
Scope: the second half of M3. The first half — a server that holds the session and a client
that holds the terminal — is in `main`; this is what makes that split worth having.

## 1. Goal and non-goals

### Goal

A session outlives the terminal that was looking at it, on purpose and by accident alike.
`C-b d` leaves it running; so does closing the terminal or losing the connection the client
was on. Running `neotmux` again shows the same panes with the same shells still in them.

### Non-goals

- **Named sessions**, `ls` and `kill-session`. The socket layer is already one file per
  session (`<session>.sock`), so more names is a later change to the CLI rather than to the
  design. There is one name today and it is `default`.
- **Two clients on one session at once.** The frame loop is a conversation between one server
  and one client, and the Lua layer lives on the client. Sharing a session is a different
  design, not a bigger version of this one. A second client is refused with a reason, which
  is what it already gets.
- **Carrying plugin state across a detach.** See §5.

## 2. What changes, in one paragraph

The server already holds the session in an `Option<Session>` and already keeps it alive while
no client is looking, for three seconds. Detach is that timeout removed and the attach path
taught a second case: a client arriving when a session already exists is shown that session
rather than given a new one. Everything else — one new op, one new end, one flag, one pid in
a file — hangs off those two.

## 3. The session's lifetime

`LINGER` goes. With a session in hand, the server's `accept` waits without a deadline; the
`STARTUP` timeout for a server nobody ever connects to stays exactly as it is, because a
server with no session is not a session worth keeping.

A session ends when, and only when:

- its last shell exits (`End::Eof`), or
- an attached client's plugin calls `neotmux.quit()` (`End::Quit`), or
- the server is signalled — `neotmux --kill` (§6) or an ordinary `SIGTERM`.

While no client is attached, the server goes on reading its children exactly as it does now,
so a pane that floods its pipe is not blocked by the absence of a screen. `forget_pane_output`
stays: nobody is subscribed to anything while nobody is running plugins.

## 4. Attaching to a session that is already there

`Attach` keeps its fields. What changes is that they are read twice over: once as the
arguments a session is created from, and once as values applied to a session that exists.

- `size` — the session is resized to the new client's terminal.
- `reserved`, `gutter` — replaced with what this client's plugins declared. The bands belong
  to the Lua layer, and the Lua layer is new.
- `wants_pane_output` — replaced for the same reason.
- `first_window` — ignored. The session's windows already have ids.

After that the server publishes the geometry, and that is the whole of what the new client is
told. `Geometry` carries `panes`, `focus`, `windows` and the window being shown, so
`pane.list()`, `window.list()` and `pane.current()` answer correctly from the first tick.

**No events are replayed.** A re-attaching plugin gets no `pane_opened` for panes that were
opened before it existed — which is the behaviour a *fresh* session already has, since the
first pane of a new session raises no event either. Geometry is how a plugin learns what is
there; events are how it learns what changed. Document it, do not special-case it.

### Window ids

`WindowIds` is a counter starting at 1, on the client, because `window.create()` must answer
with an id before the op that makes the window is applied (§4.6) — that is an ABI guarantee
`C-b c` depends on, and it is why the counter cannot simply move to the server.

A client attaching to an existing session therefore has to start past the ids already in use.
It takes the watermark from the first geometry it is published: the counter resumes at one
past the highest window id there. Ids are still never reused.

## 5. What a detach does not carry

The Lua layer is the client's. A detach ends that process; an attach starts a new one, which
loads `init.lua` from scratch.

Kept, because it is the server's: every pane, every window, every shell and what it was
running, the layout, the focus, and each pane's terminal state including its scrollback.

Lost, because it is the client's: `plugin.copy`'s register, whether the sidebar was folded,
which mode was raised, every plugin's local state, and every reservation — which is
immediately re-made by the plugin that wants it, since `reserve` runs at load.

This is stated in `docs/plugins.md` next to `neotmux.detach`, in the spec's ABI section, and
in `CHANGELOG.md`. It is a consequence of where the Lua layer lives, which is a decision M3
made deliberately: a plugin that freezes should cost one client rather than every session.

## 6. Leaving, and killing

### `neotmux.detach()`

One op and one end, alongside the two `neotmux.quit()` already uses:

- `SessionOp::Detach`, raised by `neotmux.detach()` and applied after the tick like every
  other op (§4.6).
- `End::Detach`, the server's answer. The client restores the terminal and exits 0. The
  server keeps the session and goes back to waiting for a client — that difference is the
  entire difference from `Quit`.

`PROTOCOL_VERSION` goes to 3: a variant on the wire is a wire change, and a client and server
from different commits must refuse each other by name and number rather than guess.

`runtime/lua/plugin/prefix.lua` gains `commands['d']`. There is no host keybinding, here as
everywhere.

### `neotmux --kill`

Deliberately not a protocol message. A protocol message could not be delivered in the case
that most needs it: a rebuilt binary meeting a server from the previous version refuses the
handshake, and an unreachable immortal session is the worst thing this design could ship.

The server writes its pid into the lock file it already holds (`<session>.lock`, held under
`flock` for the life of the process). `--kill` reads it and sends `SIGTERM` — but only after
finding the lock held, because a pid in a file is a hint and the lock is the identity (§7).
The server's
`ShutdownSignal` is already installed; it unbinds the socket and drops the shells, which hangs
up their children — the same exit an ordinary `SIGTERM` gives it today.

With no server running, `--kill` says so and exits non-zero. It does not remove any file: the
socket belongs to whoever bound it, and `unbind` already compares inodes rather than names.

## 7. Error handling

- **A client that dies mid-frame** is indistinguishable from one that detached, and is treated
  the same: the session is kept. This is the case the feature exists for.
- **A second client while one is attached** is turned away with the reason it already gets.
- **A version mismatch** is refused by the handshake on both sides, as now. The way out is
  `--kill`, which is why §6 keeps it off the protocol.
- **A terminal too small for the bands the new client's plugins reserve** is the existing §6
  hidden-pane case, reached through resize rather than through attach. It is not new
  behaviour and is not given new behaviour.
- **A stale lock file with a pid that is gone, or that belongs to something else now.**
  `--kill` signals a pid it read from a file; a pid file is a hint, not an identity. The lock
  is the identity: `--kill` refuses to signal unless the lock is held (a lock it can take
  itself means no server is there), and reports that rather than signalling a stranger.

## 8. Tests

Behaviour, over a real PTY, in `crates/neotmux/tests/`:

- Detaching leaves the server running, and a second `neotmux` shows the same pane with the
  same shell — asserted on output the shell produced *before* the detach, which only a kept
  terminal state can still show.
- A client killed outright, rather than detached, leaves the session in the same state.
- Attaching from a terminal of a different size refits the panes to it.
- A second client, while one is attached, is still refused with its reason.
- `--kill` ends a detached session, and its children go with it.
- `--kill` with nothing running says so and changes nothing.
- A window created before a detach and one created after do not share an id.
- The shipped status line reports the session correctly after a re-attach, which is the
  cheapest check that geometry alone is enough for a plugin to see the session.

## 9. What this does not touch

`neotmux-core`, `neotmux-term`, `neotmux-lua`'s draw and budget machinery, and every plugin
but `prefix`. The change is the server's lifetime, one branch in its attach path, one op, one
end, one flag and one pid.

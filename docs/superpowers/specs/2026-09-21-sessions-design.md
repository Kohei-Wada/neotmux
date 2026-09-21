# neotmux — sessions: names and two clients

Date: 2026-09-21
Status: proposed
Scope: the two items the detach/attach spec listed as non-goals — named sessions with a CLI
surface (§3–§5), and two clients on one session at once (§6–§7). The socket layer was already
built for both: one file per session, and the refusal the second client gets today is one
branch in the server loop.

## 1. Goal and non-goals

### Goal

`neotmux work` starts a session called `work` where nothing is running and attaches to it
where one is; `neotmux ls` shows which sessions are running; `neotmux kill work` ends one by
name. `default` keeps working exactly as it does today.

Two terminals can be attached to the same session. What one types reaches the same shell the
other looks at — tmux's arrangement — while each terminal keeps its own plugin state, because
the Lua layer is the client's and that is the line this design keeps drawing.

### Non-goals

- **Nested session names.** tmux's `work:sub` is out. Names are flat.
- **Per-client window selection.** Both clients see the session's active window; choosing a
  different one per client is `switch-client`, which is a later change.
- **Merged geometry.** A session has one surface, not one per client (§6). The rule that makes
  that workable is written down there.
- **Per-session plugin configuration.** A plugin that wants to differ per session branches on
  `neotmux.session_name()` (§5). There is no per-session `init.lua`.
- **Bounding or scheduling clients.** A frame waits for every attached client's tock; a slow
  client holds the frame (§6). Fairness machinery is not built and not needed until somebody
  asks for it.

## 2. What changes, in one paragraph

Two independent layers, landing as two stages because the second's tests want the first.

M4a makes the name first-class: `Paths::resolve` stops hard-coding `default`, the CLI gains a
positional name and `ls`, and `--kill` becomes `kill [name]`. This stage does not touch the
server loop — each session still has exactly one client at a time.

M4b rewrites the server's frame loop from a conversation with one client into a frame with
many: a second connection is adopted instead of refused, the tick and the done are fanned out
to every attached client, the tock is collected from each, and what each sends that is not a
tock is the session's, shared by the rest. The wire protocol keeps every message it has and
bumps nothing.

## 3. Names and identity (M4a)

A session name is a filename. `$XDG_RUNTIME_DIR/neotmux/<name>.sock` and `<name>.lock` are
where the name becomes a path, so the name's law is before any filesystem is touched:

- `[A-Za-z0-9_-]`, one to sixty-four characters. Nothing else. The directory is `0700` and
  ours; a name that could walk out of it does not get to exist.
- Validated at argument parsing, before any path is resolved or any file created. `neotmux ''`
  and `neotmux 'a:b'` say why they refuse; nothing is left behind.

`sock::Paths::resolve` takes the name as a parameter where `DEFAULT_SESSION` is hard-coded
today. The `flock` handshake it already runs is per name, so two `neotmux work` starting at
the same instant race exactly as two `neotmux` already do: the lock is taken, the socket is
probed, and the loser stands down. Nothing about the race changes with names.

`--server` takes the same optional name, defaulting to `default`. That is the only change the
spawn path needs: a client starting `work` spawns a server that is told to hold `work`.

## 4. The CLI surface

| invocation | meaning |
|---|---|
| `neotmux [name]` | attach to the named session if it is running, start it if it is not. Omitted: `default`, today's behaviour |
| `neotmux ls` | the running sessions, one per line, `name  pid <pid>`, sorted by name. No sessions: `no sessions`, non-zero exit |
| `neotmux kill [name]` | end the named session and its shells. Omitted: `default`. Replaces `--kill`, which is removed and noted in `CHANGELOG.md`, and takes the two out-of-scope lines out of the `README.md`'s status section when each stage lands |

`ls` is deliberately not a protocol message. `sock.rs` already says *killing one, finding one,
or listing them is `ls` rather than a protocol*; this makes the sentence true. Scanning the
socket directory and connect-probing each `<name>.sock` distinguishes running from not the
same way the client already does — a socket file with nothing behind it is not a session, and
no message crosses. The pid comes from the lock file the server writes on start.

What `ls` does not show, and why it stays cheap: it cannot see the session from the inside, so
there is no pane count, no attached/detached column, and no first-window name. Those would be
a query each server had to answer, which is the protocol the design is avoiding. A name and a
confirmed server is what can be said from the outside, and it is said from the outside.

`kill [name]` reuses the `holder()` confirmation the existing `--kill` has — the pid in the
lock file is a hint, the held lock is the identity, the pid is checked against
`/proc/<pid>/comm` before it is named — with the paths parameterised. Nothing about how a kill
is refused on a stranger's pid changes.

## 5. The Lua surface

One function, next to `neotmux.detach()`:

`neotmux.session_name()` — the name this client attached to. Answered from the client, which
knows it from its own argument; no round-trip, no protocol. A plugin that wants to behave
differently under `work` and under `default` branches on it. Nothing else is added.

## 6. One session, two clients (M4b)

The line is the one M3 drew. Shared, because it is the server's: every pane, every window,
every shell and what it is running, the layout, the focus, and each pane's terminal state
including its scrollback. Per client, because the Lua layer is the client's: every plugin's
state, its modes and registers, its bands and floats.

- **Input is shared.** A key some client's plugins did not consume goes to the focused pane,
  whichever client it came from. Type into the shell on one terminal and see it move on the
  other.
- **Output is shared.** The tick and the done of every frame reach every attached client.
- **A client's mode is its own.** Whether one terminal is in copy mode says nothing to the
  other; the second types straight through, because the first's plugins are not the second's.
  That is tmux's arrangement, and it costs nothing extra, because unconsumed was always the
  line at which a key becomes the session's.

### Geometry belongs to the session, not the client

A session has one surface. Two rules, both already the shape of the code:

- **`size`** — the last `Resize` to arrive wins. A client's `SIGWINCH` has always re-sized the
  whole session; with two clients it just does it twice. That is tmux's default `window-size`
  of `latest`, stated.
- **`reserved` and `gutter`** — a client that attaches carries its plugins' reservations, and
  `session.adopt` already applies what it carries to the whole session as it does for `size`.
  The last client to attach sets the bands everyone sees. Merging two clients' bands is not
  built.

A client whose terminal is smaller than the session's surface sees the existing
too-small-terminal behaviour — the hidden-pane case the detach/attach spec reached the same
way. No new clipping semantics.

`wants_pane_output` is the OR of what the attached clients asked, because the pipe is the
session's and nobody reading it is a shell blocked on a byte. When a client leaves, the OR is
taken over the rest; when the last one that wanted it leaves, `forget_pane_output` runs, as it
does today when the sole client does.

## 7. The frame with two clients

Today a frame is `tick`, this client's `Tock`, the `Done`, and whatever the client says until
the deadline. With two clients it is the same frame with the middle widened:

1. One `tick`, computed once, fanned out to every attached client.
2. Each attached client, in the order it attached, produces tock. While each is doing it the
   server answers that client's `Read`s on that client's link, and applies whatever `Key` or
   `Resize` arrives, since those are the session's (§6).
3. Each tock is advanced in the order collected, and the one `Done` is fanned out.
4. Until the frame's deadline: the client links, the listener and the panes' wakeup are all
   pollable, as the single link and the listener already are. A connection that arrives is
   greeted, its `Attach` read, adopted through the path that exists for re-attaching, and
   added — the place today refuses it with a reason, and it now keeps the connection.

Collecting each client's tock in attach order rather than arrival order is deliberate: two
clients' tocks applied in the order the sockets happened to wake up are the same work in
another order every run, and the tests want the one that is the same.

What does not change, and must not:

- **Nothing a client does to its own connection ends the session** — the rule `attached`
  already keeps. Extended per link: undecodable bytes or a hangup on one client's link drops
  that client only, and the rest of the frame goes on for the others.
- **`End::Quit` and `End::Eof` end the session from whichever client they arrive on**;
  `End::Detach` still means only this client leaves, and a session with no client in it is
  exactly the case the server already serves.
- **The `STARTUP` timeout** still applies only to a server that never got a session.
- **Window ids.** Each client starts its counter past the watermark of the first geometry it
  is published, as a re-attaching client already does. Two clients keeping their own counters
  is the existing rule doing its job.

The wire protocol gains no message and loses none. `Refused` stays, for the cases that still
refuse — a handshake that fails, a version that does not match; the `BUSY` that a second
client used to get is the one thing it stops getting. No protocol version is bumped, because
the version is for the shape of messages and no shape changed.

## 8. Error handling

- **A second client arriving mid-frame** is adopted between frames the way it is handled today
  — `converse` already reaches the listener inside a frame, which is how the refusal ships.
  The new client's first done is the next frame, and it learns the session from the geometry
  it is published, the same way a re-attaching client does. No events are replayed to it, for
  the reason the detach/attach spec gives.
- **A client that dies while another is attached** costs the session nothing that a client
  dying today costs it more than: the same detach path, the OR of `wants_pane_output` taken
  over the rest, the frame fanned out to the rest.
- **A client whose plugins blow their tick** holds the frame until it answers or goes
  (§1, non-goals). It is the existing per-tick budget on the client's side that bounds this;
  what the frame does not do is guess at a tock for a client that is not sending one.
- **Killing the session** kills every client at once, the way it kills the one client today.
  `kill [name]` is a signal at the same distance, with the same stranger-pid refusal.
- **Everything `ls` could be told a lie by** is one class: a socket that answers but is not
  this user's is refused by the socket's mode and the directory's law, as it is for
  `default`. A name that is a broken symlink into the directory is not a file and not a
  session; `ls` connect-probes the file it sees, and what answers is what it says.

## 9. What the client changes

`start.rs` resolves paths for the name it was given and passes it on when spawning. The attach
path, the handshake, the Lua loading and every plugin keep their shape. A client receiving
`Refused` keeps the handling it has; what changes is that attaching a second time to a named
session stops being the case that gets it.

`neotmux.session_name()` is the one new ABI, and it is answered from the client, so
`docs/plugins.md` and the ABI section gain one line for it.

## 10. Tests

Stage one, over a scratch `XDG_RUNTIME_DIR`:

- `neotmux work` with nothing running starts it; `ls` shows `work` and its confirmed pid, and
  shows `default` only if `default` is running too.
- A second `neotmux work` shows what the first one already produced — the kept terminal state
  assertion the detach/attach tests already use, with a name on it.
- `kill work` ends `work` and its children and leaves `default` running; `kill` on a name
  nothing is running says so and exits non-zero.
- Bad names are refused at the argument: nothing appears in the scratch directory.
- `session_name()` answers `work` to a plugin running under `work`.

Stage two, two real client connections against one session, in `crates/neotmux/tests/`:

- Both clients get the tick and the done of the same frame.
- A key typed on one client moves the focused pane, and the other client's next done shows it.
- A `Read` from one client is answered on that client's link and nowhere else.
- A client dying mid-frame — unreadable bytes on its link, then a hangup — drops its
  connection only; the other client's frames go on.
- A second client adopted while one is attached learns the session from its published geometry,
  and the last client to attach sets the bands.
- `wants_pane_output` is the OR: a pane is delivered while the client that wanted it is
  attached, and stops with it, while a client that did not want it is still attached.
- Over a real PTY, in two terminals on `neotmux work`: text typed in one is in the shell on
  the other.

## 11. What this does not touch

`neotmux-proto`'s message set and its version, `neotmux-core`'s layout, the budget and draw
machinery in `neotmux-lua`, and the race law in `sock.rs` — the name is threaded through the
paths it already governs. The ABI gains one function the client answers. Everything else is
the server loop widened, one parameter on `Paths`, and a CLI row.

# The Lua VM Moves to the Server — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `LuaSubsystem` in the server process, so a plugin runs beside the panes it
talks about and goes on running when no terminal is attached.

**Architecture:** The server gains the Lua layer, the plugin registry, key dispatch, the
overlay and the notices; the client keeps raw mode, key decoding, painting and `SIGWINCH`.
The four-message frame (`Tick`/`Tock`/`Read`/`Reading`) becomes four function calls inside
one process; what is left on the wire is `Attach { size }`, `Key`, `Resize` up and a composed
frame down. Two side effects that need a terminal — putting text on the clipboard and
replaying the notification transcript on exit — become two new server-to-client messages.

**Tech Stack:** Rust 2024, one workspace of five crates (`neotmux-proto`, `neotmux-core`,
`neotmux-term`, `neotmux-lua`, `neotmux`), `mlua` (Lua 5.4), `ratatui`, `nix`.

**Spec:** `docs/superpowers/specs/2026-09-23-lua-on-the-server-design.md` — this plan is
stage 2 of its §10. Stage 1 (the budget, §4) landed in commit `60deea4`.

## Global Constraints

- `neotmux-lua` keeps `neotmux-proto` as its only workspace dependency, beside `anyhow` and
  `mlua` (spec §12). Nothing in this plan adds a dependency to it.
- One Lua VM per server, which is to say one per session (spec §7). Never one per client.
- No blocking host call on the Lua API (spec §4): anything that would wait is a subscription
  delivered on a later tick, never a call that returns when the waiting is over.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` are all green at every commit. A task is not done until its
  author has read that output.
- English for code, comments, docs and commit messages.
- No comment unless the WHY is non-obvious, and then one line. No backward-compatibility
  shims and no `// removed` markers: deleted code is deleted.
- Commit messages are lower-case sentences in this repo's voice (see `git log`), and end with
  the two attribution lines the repo's recent commits carry.

## Rulings recorded before execution

The spec's §10 leaves four things to whoever plans this. They are decided here, so no task
has to guess.

1. **`Attach` shrinks in this stage, not in the sweep.** §10 files the shrink under stage 3,
   but a client with no Lua layer cannot fill `first_window`, `reserved`, `gutter` or
   `wants_pane_output`. The shrink is forced by the move, so it happens here, and
   `PROTOCOL_VERSION` goes up with it. What stays until stage 3 is the *dead* wire vocabulary
   — `Tick`, `Tock`, `Step`, `Read`, `Reading`, `reads.rs` — which nothing in this stage has
   to touch to be correct.
2. **The clipboard gets a message.** §6's protocol table does not list one, and it has to:
   `neotmux.clipboard.set` writes OSC 52 to a terminal, and after this change the code that
   calls it is in the process that has none. `ToClient::Clip` carries the text to every
   attached client, which writes the sequence to its own terminal. With one client — the
   ordinary case — that is exactly today's behaviour. A yank with nobody attached is refused,
   with a reason, rather than queued: `Clipboard::set` promises the host did what it does.
3. **The exit transcript gets a message.** `docs/plugins.md` promises notifications "replay on
   stderr when neotmux exits, so a message from a frame you never saw is not lost". The
   transcript moves to the server with the `Notifications` that hold it, and the server's
   stderr is `/dev/null`. `ToClient::Bye` carries the lines raised while *that* client was
   attached, and the client prints them once the terminal is restored.
4. **`neotmux.detach()` detaches every client.** Today the op comes from one client's own Lua
   layer, so it is that client that leaves. With one shared VM there is no attribution to
   make: the op ends every attached client and keeps the session. With one client that is
   today's behaviour. Attributing an op to the client whose key raised it is what spec §9's
   command surface will make answerable, and is not built here.

---

## File Structure

| File | What it is responsible for afterwards |
| --- | --- |
| `crates/neotmux-proto/src/msg.rs` | `Attach { size }`; the two new `ToClient` variants. |
| `crates/neotmux-proto/src/wire.rs` | Their encoding; the shrunken `Attach`'s. |
| `crates/neotmux-proto/src/lib.rs` | `PROTOCOL_VERSION`. |
| `crates/neotmux/src/session.rs` | Unchanged behaviour; starts from a `Setup` it is handed and from terminals it is handed, rather than from an `Attach` and terminals of its own. |
| `crates/neotmux/src/clip.rs` | **New.** Where a yank waits for a client to write it to. |
| `crates/neotmux/src/notify.rs` | The transcript, replayable from a mark. |
| `crates/neotmux/src/plugins.rs` | Unchanged code, corrected prose: this is the server's half now. |
| `crates/neotmux/src/server.rs` | The frame: tick the session, run the Lua layer, apply, paint every client. Key dispatch. The door. |
| `crates/neotmux/src/tui.rs` | Keys up, frames down. No Lua. |
| `crates/neotmux/src/main.rs` | A client that starts a server and connects to it. |
| `crates/neotmux/tests/socket.rs` | The protocol as it is now; four tests deleted, one rewritten, one added. |
| `docs/plugins.md` | Where plugin code runs and what that means for the clipboard. |

---

### Task 1: The session is handed its terminals and a `Setup`

Today `Session::new` takes a `&Attach` off the wire and makes its own `PaneTerminals`. The
server is about to need those terminals *before* the session exists — the Lua layer reads
panes through them and is built first — and to fill the four Lua-owned fields itself. This
task makes both possible and changes no behaviour.

**Files:**
- Modify: `crates/neotmux/src/session.rs`
- Modify: `crates/neotmux/src/server.rs:400-425` (`admit`)
- Modify: `crates/neotmux/tests/headless.rs:30-56`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `neotmux::session::Setup { size: Size, first_window: WindowId, reserved: Reserved, gutter: u16, wants_pane_output: bool }`;
  `Session::new(setup: &Setup, waker: Waker, terminals: Rc<PaneTerminals>) -> anyhow::Result<Session>`;
  `Session::adopt(&mut self, setup: &Setup) -> anyhow::Result<()>`.

- [ ] **Step 1: Add `Setup` to `session.rs`**

Above `impl Session`:

```rust
/// What a session is started with, or refitted to.
///
/// The size is the attaching client's, because the terminal is. The other four are the Lua
/// layer's — the window id counter, the bands the plugins reserved, the gutter between panes
/// and whether anything is watching pane output — and this file has no way to reach it.
pub struct Setup {
    pub size: Size,
    pub first_window: WindowId,
    pub reserved: Reserved,
    pub gutter: u16,
    pub wants_pane_output: bool,
}
```

- [ ] **Step 2: Take the terminals and the `Setup` in `new`**

Replace the signature and the two lines that used the attach:

```rust
    pub fn new(
        setup: &Setup,
        waker: Waker,
        terminals: Rc<PaneTerminals>,
    ) -> anyhow::Result<Self> {
        let surface = Surface {
            size: clamp_to_min_grid(setup.size),
            reserved: setup.reserved,
            gutter: setup.gutter,
            minimum: neotmux_term::state::MIN_GRID,
        };
        let (windows, first) = Windows::new(setup.first_window, surface);
        let mut shells = Shells::new(waker, terminals.clone());
```

The `let terminals = Rc::new(PaneTerminals::new());` line goes; `wants_pane_output:
attach.wants_pane_output` becomes `setup.wants_pane_output`. The doc comment's "all four come
from the client, because all four are the client's to know" is no longer true — replace it
with "the size is the client's; the other four are the Lua layer's, and the caller is where
it can be reached".

- [ ] **Step 3: Take the `Setup` in `adopt`**

`pub fn adopt(&mut self, setup: &Setup) -> anyhow::Result<()>`, with `attach.` → `setup.`
throughout its body. Its "`first_window` is ignored — the windows already have ids" comment
stays true and stays.

- [ ] **Step 4: Build the `Setup` at the door**

In `server.rs`'s `admit`, before the match:

```rust
    let setup = Setup {
        size: attach.size,
        first_window: attach.first_window,
        reserved: attach.reserved,
        gutter: attach.gutter,
        wants_pane_output: attach.wants_pane_output,
    };
    match session.as_mut() {
        Some(session) => session.adopt(&setup)?,
        None => {
            *session = Some(Session::new(
                &setup,
                waker.clone(),
                std::rc::Rc::new(neotmux_term::terminals::PaneTerminals::new()),
            )?);
        }
    }
```

Task 4 hoists that `PaneTerminals::new()` out to where the Lua layer can share it; here it
stays inline so this task changes nothing.

- [ ] **Step 5: Fix `headless.rs`**

`halves()` builds the session first and the Lua layer second. Build the terminals first
instead, so the fixture is the shape the server will have:

```rust
    let (wakeup, waker) = wakeup().expect("wakeup");
    let terminals = std::rc::Rc::new(neotmux_term::terminals::PaneTerminals::new());
    let session = Session::new(
        &Setup {
            size: SURFACE,
            first_window: WindowId::from(1),
            reserved: Reserved::default(),
            gutter: 0,
            wants_pane_output: false,
        },
        waker,
        terminals.clone(),
    )
    .expect("start a session");
    let lua = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        terminals,
    )
    .expect("build the Lua subsystem");
```

The `use neotmux_proto::msg::{Attach, End};` import loses `Attach` and gains
`neotmux::session::Setup` from the session import.

- [ ] **Step 6: Run the suite**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: everything that passed before passes. This task has no test of its own: it moves
where two values come from and nothing else, and the suite is what says so.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux/src/session.rs crates/neotmux/src/server.rs crates/neotmux/tests/headless.rs
git commit -m "a session is handed its terminals rather than making them"
```

---

### Task 2: The transcript can be replayed from a mark

The notification transcript is about to become the session's rather than one client's. A
client that attaches to a session that has been running for an hour must be told what was
raised while *it* was there, not the hour's worth.

**Files:**
- Modify: `crates/neotmux/src/notify.rs`
- Test: `crates/neotmux/src/notify.rs` (its own `mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: `Notifications::mark(&self) -> usize`, `Notifications::since(&self, mark: usize) -> Vec<String>`.

- [ ] **Step 1: Write the failing tests**

Into `mod tests` in `notify.rs`:

```rust
    #[test]
    fn a_mark_replays_only_what_was_raised_after_it() {
        let mut n = Notifications::new();
        n.push(Level::Info, "a", "one");
        let mark = n.mark();
        n.push(Level::Info, "b", "two");
        assert_eq!(n.since(mark), vec!["b: two".to_string()]);
        assert_eq!(
            n.since(0),
            vec!["a: one".to_string(), "b: two".to_string()],
            "a mark of zero is the whole of it"
        );
    }

    /// A mark older than the oldest line still held is not an index into nothing: what is
    /// left of the transcript is what that client is told.
    #[test]
    fn a_mark_the_cap_has_overtaken_replays_what_is_left() {
        let mut n = Notifications::new();
        for i in 0..MAX_TRANSCRIPT + 3 {
            n.push(Level::Info, "loud", &format!("line {i}"));
        }
        let lines = n.since(0);
        assert_eq!(lines.len(), MAX_TRANSCRIPT);
        assert_eq!(lines[0], "loud: line 3");
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p neotmux --lib notify`
Expected: FAIL, `no method named 'mark' found`.

- [ ] **Step 3: Implement**

Add the counter to the struct:

```rust
pub struct Notifications {
    items: Vec<Notification>,
    transcript: VecDeque<String>,
    dropped: usize,
    /// Everything ever pushed, the dropped included. A client's mark is one of these, so a
    /// mark stays meaningful after the cap has thrown the line it pointed at away.
    pushed: usize,
}
```

In `push`, straight after `self.transcript.push_back(...)`: `self.pushed += 1;`.

And the two methods, beside `flush`:

```rust
    /// Where the transcript is now: what a client attaching takes, and gives back on its
    /// way out.
    pub fn mark(&self) -> usize {
        self.pushed
    }

    /// Everything raised since `mark`, as `flush` would write it, without the `neotmux: `
    /// each line is printed behind.
    pub fn since(&self, mark: usize) -> Vec<String> {
        let oldest = self.pushed - self.transcript.len();
        let from = mark.saturating_sub(oldest);
        self.transcript.iter().skip(from).cloned().collect()
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux --lib notify`
Expected: PASS, the three existing `flush` tests included.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/src/notify.rs
git commit -m "a transcript that can be replayed from where a client came in"
```

---

### Task 3: A clipboard the server can hold

`neotmux.clipboard.set` writes OSC 52 to a terminal. After Task 4 the code that calls it runs
in the process with no terminal, so the text has to wait for a frame to hand it to the
clients. This task builds that holding place and tests it on its own.

**Files:**
- Create: `crates/neotmux/src/clip.rs`
- Modify: `crates/neotmux/src/lib.rs` (`pub mod clip;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `neotmux::clip::Clip`, with `Clip::new()`, `Clip::attached(&self, count: usize)`,
  `Clip::take(&self) -> Vec<String>`, and `impl neotmux_lua::clipboard::Clipboard for Clip`.

- [ ] **Step 1: Write the failing tests**

Create `crates/neotmux/src/clip.rs` with the module doc, the tests, and nothing else yet:

```rust
//! Where a yank waits, now that the Lua layer is in the process with no terminal.
//!
//! OSC 52 is a sequence written to a terminal, and the server's terminals are its clients'.
//! So `neotmux.clipboard.set` queues the text here and the frame hands it to whoever is
//! attached, which is the one place that knows who that is.

#[cfg(test)]
mod tests {
    use super::*;
    use neotmux_lua::clipboard::Clipboard as _;

    #[test]
    fn a_yank_with_a_client_attached_is_queued_once_and_taken_once() {
        let clip = Clip::new();
        clip.attached(1);
        clip.set("hello").expect("a client is attached");
        assert_eq!(clip.take(), vec!["hello".to_string()]);
        assert!(clip.take().is_empty(), "taking it twice sends it twice");
    }

    /// `Clipboard::set` promises the host did what it does. With no terminal to write to
    /// there is nothing it does, and saying so is the honest answer — the Lua side turns it
    /// into the `false` a plugin can act on.
    #[test]
    fn a_yank_with_nobody_attached_is_refused_rather_than_held() {
        let clip = Clip::new();
        clip.attached(0);
        assert!(clip.set("hello").is_err());
        assert!(clip.take().is_empty());
    }
}
```

- [ ] **Step 2: Wire the module in and watch it fail**

Add `pub mod clip;` to `crates/neotmux/src/lib.rs`, beside `pub mod decode;`.

Run: `cargo test -p neotmux --lib clip`
Expected: FAIL, `cannot find type 'Clip' in this scope`.

- [ ] **Step 3: Implement**

Above the test module:

```rust
use std::cell::{Cell, RefCell};

#[derive(Default)]
pub struct Clip {
    queued: RefCell<Vec<String>>,
    attached: Cell<usize>,
}

impl Clip {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many clients there are to write to, as of this frame.
    pub fn attached(&self, count: usize) {
        self.attached.set(count);
    }

    /// Everything yanked since the last frame took it.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.queued.borrow_mut())
    }
}

impl neotmux_lua::clipboard::Clipboard for Clip {
    fn set(&self, text: &str) -> std::io::Result<()> {
        if self.attached.get() == 0 {
            return Err(std::io::Error::other(
                "no client is attached, so there is no terminal to write it to",
            ));
        }
        self.queued.borrow_mut().push(text.to_string());
        Ok(())
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux --lib clip`
Expected: PASS, both.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/src/clip.rs crates/neotmux/src/lib.rs
git commit -m "somewhere for a yank to wait for a terminal"
```

---

### Task 4: The VM moves

The switch. Both ends of the protocol change at once, because there is no state between the
runtime being on one side and being on the other. This is the task the spec's §10 says needs
its own decomposition; Tasks 1-3 are that decomposition's preparation, and what is left here
is the move itself.

**Files:**
- Modify: `crates/neotmux-proto/src/lib.rs:47` (`PROTOCOL_VERSION`)
- Modify: `crates/neotmux-proto/src/msg.rs:20-36` (`Attach`), `:193-224` (`ToClient`, `OwnedToClient`)
- Modify: `crates/neotmux-proto/src/wire.rs:626-635, 680-770` (tags and codecs), and its round-trip tests
- Modify: `crates/neotmux/src/server.rs`
- Modify: `crates/neotmux/src/plugins.rs` (prose only)
- Modify: `crates/neotmux/src/tui.rs`
- Modify: `crates/neotmux/src/main.rs`
- Modify: `crates/neotmux/src/start.rs` (`server::serve` gains the session's name)
- Modify: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: `session::Setup` and the terminals parameter (Task 1); `Notifications::mark`/`since`
  (Task 2); `clip::Clip` (Task 3).
- Produces: `server::serve(name: &str, listener: &UnixListener, shutdown: &ShutdownSignal) -> anyhow::Result<()>`;
  `tui::run(terminal: &mut DefaultTerminal, shutdown: &ShutdownSignal, link: &Rc<RefCell<Link>>, size: Size) -> anyhow::Result<tui::Left>`;
  `tui::Left { signal: Option<i32>, transcript: Vec<String> }`.

- [ ] **Step 1: Shrink `Attach` and add the two messages**

In `msg.rs`:

```rust
/// What the client tells the server when it takes the session over: how big its terminal is,
/// which is the one thing about the session only it knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attach {
    pub size: Size,
}
```

The fields that go were the Lua layer's, and the Lua layer is the server's now. `WindowId` and
`Reserved` may fall out of the file's `use` — let the compiler say.

In `ToClient`, beside `Refused`:

```rust
    /// Text a plugin yanked, for the client to put on its terminal's clipboard. It is the
    /// one thing the Lua layer does that needs a terminal, and after the runtime moved into
    /// the server the terminal is on the other side of the socket.
    Clip(&'a str),
    /// The notifications raised while this client was attached, for it to replay on stderr
    /// once its terminal is restored. Sent before the frame that carries the end.
    Bye(&'a [String]),
```

and in `OwnedToClient`: `Clip(String)`, `Bye(Vec<String>)`.

- [ ] **Step 2: Encode them**

In `wire.rs`, beside the other tags:

```rust
const TO_CLIENT_CLIP: u8 = 4;
const TO_CLIENT_BYE: u8 = 5;
```

In `encode_to_client`:

```rust
        ToClient::Clip(text) => {
            w.u8(TO_CLIENT_CLIP);
            w.str(text);
        }
        ToClient::Bye(lines) => {
            w.u8(TO_CLIENT_BYE);
            w.len(lines.len());
            for line in *lines {
                w.str(line);
            }
        }
```

In `decode_to_client`:

```rust
        TO_CLIENT_CLIP => OwnedToClient::Clip(r.string()?),
        TO_CLIENT_BYE => {
            let n = r.len()?;
            let mut lines = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                lines.push(r.string()?);
            }
            OwnedToClient::Bye(lines)
        }
```

`encode_to_server`/`decode_to_server`'s `Attach` arm loses four reads and four writes, keeping
`w.size` / `r.size()?`.

Bump `PROTOCOL_VERSION` in `crates/neotmux-proto/src/lib.rs` from `3` to `4`. The test at
`wire.rs:2035` exists to say that a wire change moves it; read its message and make sure what
it asserts is still what this change did.

- [ ] **Step 3: Round-trip the new messages**

In `wire.rs`'s tests, beside the existing `round_trip_to_server` of an `Attach` (line ~1970),
which loses its four fields:

```rust
    /// Encoded and decoded inline, the way the `Refused` case beside it is: there is no
    /// `round_trip_to_client` helper, because `ToClient` borrows and `OwnedToClient` does
    /// not, so the two sides of a round trip are not the same type.
    #[test]
    fn the_clipboard_and_the_goodbye_round_trip() {
        let bytes = encode_to_client(&ToClient::Clip("yanked 日本語"));
        assert_eq!(
            decode_to_client(&bytes),
            Ok(OwnedToClient::Clip("yanked 日本語".to_string()))
        );

        let lines = vec!["a: one".to_string(), String::new()];
        let bytes = encode_to_client(&ToClient::Bye(&lines));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Bye(lines)));

        let bytes = encode_to_client(&ToClient::Bye(&[]));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Bye(Vec::new())));
    }
```

Put it beside the existing `ToClient::Refused` round trip (around line 2269), inside the same
test module. The `an_attach_round_trips` test above it loses its four fields.

Run: `cargo test -p neotmux-proto`
Expected: PASS.

- [ ] **Step 4: The server builds the Lua layer**

`server.rs`'s imports gain `std::rc::Rc`, `neotmux_lua::{KeyOutcome, LuaSubsystem}`,
`neotmux_lua::runtime::Layers`, `neotmux_term::terminals::PaneTerminals`,
`neotmux::clip::Clip`, `neotmux::plugins::Plugins`, `neotmux::session::{Pass, Setup}`.

`Attached` keeps three fields:

```rust
struct Attached {
    link: Link,
    /// Where the session's transcript was when this client attached: what it is told on its
    /// way out is what was raised while it was here.
    mark: usize,
    /// The client has gone, in this frame. Recorded rather than dropped, because the frame's
    /// slot is still a slot the wait is waiting over.
    gone: bool,
}
```

`wants`, `last_overlay`, `last_notices`, `first`, `overlay_dirty` and `notices_dirty` all go:
the overlay and the notices are the session's now, one VM's, so the picture is one picture,
and `Session::advance` already tracks whether it changed. A client that has just attached is
covered by `adopt`, which sets `dirty`.

`serve` takes the session's name and builds the runtime before anything else:

```rust
pub fn serve(
    name: &str,
    listener: &UnixListener,
    shutdown: &ShutdownSignal,
) -> anyhow::Result<()> {
    listener
        .set_nonblocking(true)
        .context("make the socket pollable")?;
    let (wakeup, waker) = wakeup().context("open the panes' wakeup")?;
    let serving = Serving {
        listener,
        wakeup: &wakeup,
        waker: &waker,
        shutdown,
    };

    // Before the first client and before the session: the first window's id, the bands and
    // the gutter are the Lua layer's to say, and the panes it reads are the ones the session
    // is about to be given.
    let terminals = Rc::new(PaneTerminals::new());
    let clip = Rc::new(Clip::new());
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
    let layers = Layers::resolve(&|key| std::env::var_os(key), exe_dir.as_deref());
    let lua = LuaSubsystem::build_named(name, layers, terminals.clone(), clip.clone())?;
    let mut plugins = Plugins::new(lua);
    for failure in plugins.lua().run_init() {
        logging::log(Level::Warn, &failure.source, &failure.message);
    }
    // A plugin that called `neotmux.quit()` while loading has said there is to be no session.
    // The server stays up only long enough to tell whoever started it why: one that simply
    // exited would be a client waiting out its whole start timeout for a socket that never
    // answers.
    if plugins.drain_load_ops() {
        return refuse_everyone(
            &serving,
            "a plugin called neotmux.quit() while loading; no session was started",
        );
    }

    let mut session: Option<Session> = None;
    let mut clients: Vec<Attached> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    let born = Instant::now();

    loop {
        if shutdown.raised().is_some() {
            return Ok(());
        }
        if clients.is_empty()
            && !await_client(
                &mut clients,
                &mut pending,
                &mut plugins,
                &mut session,
                &serving,
                born,
            )?
        {
            return Ok(());
        }
        if frame(
            &mut session,
            &mut plugins,
            &clip,
            &mut clients,
            &mut pending,
            &serving,
        )? {
            return Ok(());
        }
    }
}
```

`await_client` and `door` take `plugins: &mut Plugins` and pass it on to `admit`; `await_client`
otherwise stays as it is until Task 5. `door`'s trailing `wants_pane_output` OR block goes —
the session learns what is subscribed from the tock, every frame, and there is no second Lua
layer to OR against.

And the refusal:

```rust
/// A server that has nothing to serve, staying up only long enough to say so. Whoever started
/// it is waiting on the socket right now; everyone after that gets the same sentence.
fn refuse_everyone(serving: &Serving<'_>, why: &str) -> anyhow::Result<()> {
    let give_up = Instant::now() + STARTUP;
    while Instant::now() < give_up {
        if serving.shutdown.raised().is_some() {
            return Ok(());
        }
        match taken(serving.listener)? {
            Some(stream) => {
                if let Ok(mut link) = Link::greet(stream) {
                    let _ = link.send(&wire::encode_to_client(&ToClient::Refused(why)));
                }
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    Ok(())
}
```

- [ ] **Step 5: The door fills the `Setup` from the Lua layer**

```rust
fn admit(
    link: Link,
    attach: &Attach,
    plugins: &Plugins,
    session: &mut Option<Session>,
    terminals: &Rc<PaneTerminals>,
    waker: &Waker,
) -> anyhow::Result<Attached> {
    let lua = plugins.lua();
    let setup = Setup {
        size: attach.size,
        first_window: lua.reserve_window_id(),
        reserved: lua.reserved(),
        gutter: lua.opt().pane_gutter,
        wants_pane_output: lua.has_subscriber("pane_output"),
    };
    match session.as_mut() {
        Some(session) => session.adopt(&setup)?,
        None => *session = Some(Session::new(&setup, waker.clone(), terminals.clone())?),
    }
    Ok(Attached {
        link,
        mark: plugins.notifications().mark(),
        gone: false,
    })
}
```

`terminals` reaches `admit` through `door`; add it to `Serving` rather than threading a sixth
parameter through two call sites:

```rust
struct Serving<'a> {
    listener: &'a UnixListener,
    wakeup: &'a Wakeup,
    waker: &'a Waker,
    shutdown: &'a ShutdownSignal,
    /// The panes' terminal state, made before the session so the Lua layer can read it.
    terminals: &'a Rc<PaneTerminals>,
}
```

- [ ] **Step 6: The frame runs the Lua layer**

Replace `frame`'s body from the tick down to the per-client picture. The tock wait is gone
entirely — there is nobody to wait for.

```rust
fn frame(
    session: &mut Option<Session>,
    plugins: &mut Plugins,
    clip: &Clip,
    clients: &mut Vec<Attached>,
    pending: &mut Vec<Pending>,
    serving: &Serving<'_>,
) -> anyhow::Result<bool> {
    door(pending, clients, plugins, session, serving)?;
    clip.attached(clients.len());

    let Some(live) = session.as_mut() else {
        clients.retain(|client| !client.gone);
        return Ok(false);
    };

    // The whole of what used to be four messages: the session says what happened, the Lua
    // layer answers, the session applies it, and the picture comes back.
    let tick = live.tick()?;
    let tock = plugins.respond(tick);
    let Pass { steps, frame, end } = live.advance(&tock)?;
    let picture = (frame.is_some() || end.is_some()).then(|| {
        wire::encode_to_client(&ToClient::Done(&Done {
            // Nothing on the client has to be caught up any more: the steps are applied to a
            // Lua layer in this process, below.
            steps: Vec::new(),
            frame: frame.as_ref(),
            end,
        }))
    });
    drop(frame);

    let yanked: Vec<Vec<u8>> = clip
        .take()
        .iter()
        .map(|text| wire::encode_to_client(&ToClient::Clip(text)))
        .collect();

    for client in clients.iter_mut().filter(|client| !client.gone) {
        for bytes in &yanked {
            if client.link.send(bytes).is_err() {
                client.gone = true;
            }
        }
        // Before the frame that carries the end, so a client that reads its end and stops
        // reading has already been told what to say.
        if end.is_some() && !client.gone {
            let lines = plugins.notifications().since(client.mark);
            if client
                .link
                .send(&wire::encode_to_client(&ToClient::Bye(&lines)))
                .is_err()
            {
                client.gone = true;
            }
        }
        if let Some(bytes) = &picture
            && !client.gone
            && client.link.send(bytes).is_err()
        {
            client.gone = true;
        }
    }
    plugins.catch_up(steps);
    clients.retain(|client| !client.gone);

    match end {
        // The session is kept and every client leaves: with one VM there is no client the
        // op belongs to more than another. See the plan's ruling 4.
        Some(End::Detach) => {
            clients.clear();
            return Ok(false);
        }
        Some(_) => return Ok(true),
        None => {}
    }
```

The rest of the frame — the read loop that fills the remainder of `FRAME` — keeps its shape,
and its `Key` arm gains the dispatch the client used to do:

```rust
                            Ok(ToServer::Key(key)) => {
                                // Offered to the plugins first and passed on only if nobody
                                // wanted it: which pane an unbound key belongs to is the
                                // session's to know, and both halves are here now.
                                if plugins.key(key) == KeyOutcome::Fallthrough {
                                    let Some(session) = session.as_mut() else {
                                        continue;
                                    };
                                    if session.key(key).is_err() {
                                        clients[i].gone = true;
                                    }
                                }
                            }
```

The `Tock` arm and the trailing `wants_pane_output` OR block go. `Read` stays: nothing sends
one any more, but the arm and `reads.rs` are the sweep's (stage 3), not this task's, and the
test that pins the answer still passes.

- [ ] **Step 7: `start.rs` passes the name**

```rust
    let served = server::serve(name, &listener, &shutdown);
```

- [ ] **Step 8: The client stops running Lua**

`main.rs`: delete `TerminalClipboard` and the `neotmux_lua`, `Plugins`, `Layers`, `reads`
imports it and `client()` used. `client()` becomes:

```rust
fn client(session: Option<String>) -> anyhow::Result<()> {
    // First, and before the socket: until the terminal is in raw mode the line discipline is
    // answering the keyboard, and starting a server is not fast.
    let _raw = RawMode::enter();
    let link = start::begin(session)?.finish()?;

    let shutdown = signals::ShutdownSignal::install()?;
    let mut terminal = ratatui::init();
    let size = tui::terminal_size(&terminal)?;
    let result = tui::run(&mut terminal, &shutdown, &link, size);
    // Not `ratatui::restore`: it reports a failure with `eprintln!`, which panics when the
    // terminal it is trying to restore is the one that has gone away.
    if let Err(err) = ratatui::try_restore() {
        logging::log(
            Level::Warn,
            "neotmux",
            &format!("could not restore the terminal: {err}"),
        );
    }
    let left = result?;
    for line in &left.transcript {
        eprintln!("neotmux: {line}");
    }
    match left.signal {
        Some(signal) => signals::die_of(signal),
        None => Ok(()),
    }
}
```

`start::begin` and `Starting::finish` stay as they are. They were split so the client could
load its plugins while the server started; there is nothing to do in that window now, and the
split costs nothing, so leave it — removing it is not this task's business.

- [ ] **Step 9: The client's loop is keys up and frames down**

In `tui.rs`, `attach()` and `round()` go, and `run` becomes:

```rust
/// Why the client stopped, and what the session asked it to say once the terminal is back.
pub struct Left {
    pub signal: Option<i32>,
    pub transcript: Vec<String>,
}

/// The process with the screen: the terminal, the raw mode, the key decoding, the drawing.
///
/// It holds no plugin and makes no decision. A key it reads is sent, not judged; the picture
/// that comes back is painted, not composed.
pub fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
    link: &Rc<RefCell<Link>>,
    size: Size,
) -> anyhow::Result<Left> {
    let mut keyboard = Keyboard::open()?;
    let winch = signals::WindowChange::install()?;
    send(link, &ToServer::Attach(Attach { size }))?;
    let mut transcript = Vec::new();

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Left {
                signal: Some(signal),
                transcript,
            });
        }
        if winch.taken() {
            let size = terminal_size(terminal)?;
            send(link, &ToServer::Resize(size))?;
        }

        let deadline = Instant::now() + PATIENCE;
        let ready = {
            let peer = link.borrow();
            keyboard
                .tty
                .wait_beside(peer.as_fd(), deadline)
                .context("wait for input")?
        };
        match ready {
            Ready::HangUp => {
                return Ok(Left {
                    signal: Some(signals::HANGUP),
                    transcript,
                });
            }
            Ready::Input => match keyboard.typed()? {
                Typed::HangUp => {
                    return Ok(Left {
                        signal: Some(signals::HANGUP),
                        transcript,
                    });
                }
                Typed::Keys(keys) => {
                    for key in keys {
                        send(link, &ToServer::Key(key))?;
                    }
                }
            },
            Ready::Output => {
                let mut peer = link.borrow_mut();
                if !peer.fill().context("read from the server")? {
                    return Err(gone(&mut peer));
                }
            }
            Ready::Idle => {}
        }

        loop {
            let message = link.borrow_mut().take().context("read from the server")?;
            let Some(message) = message else { break };
            match wire::decode_to_client(&message).context("decode a server message")? {
                OwnedToClient::Done(done) => {
                    if let Some(frame) = &done.frame {
                        draw::show(terminal, &frame.frame())?;
                    }
                    if done.end.is_some() {
                        return Ok(Left {
                            signal: None,
                            transcript,
                        });
                    }
                }
                OwnedToClient::Clip(text) => yank(&text),
                OwnedToClient::Bye(lines) => transcript = lines,
                OwnedToClient::Refused(why) => anyhow::bail!("{why}"),
                OwnedToClient::Tick(_) | OwnedToClient::Reading(_) => {}
            }
        }
    }
}

/// The one thing on the screen the session cannot do for itself: OSC 52, written to this
/// client's terminal and flushed. Flushing is not optional — the sequence carries no
/// newline, so a line-buffered terminal would hold it until something else came along.
fn yank(text: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = out
        .write_all(&neotmux_term::osc52::set(text))
        .and_then(|()| out.flush());
}
```

The `settled` flag goes with the tick: it existed so a key typed before the first tick could
not reach an `on_key` handler that believed the session was empty, and `Session::new` already
publishes the geometry before the first key for exactly that reason, in the process that now
runs the handler.

- [ ] **Step 10: `plugins.rs` says where it is**

Its module doc opens "M3 puts the Lua layer beside the screen rather than beside the panes".
That is the decision this change reverses. Rewrite the doc to say what is now true: this is
the session's half of the frame, running beside the panes, and what it costs is what spec §4
bounds. `Plugins`'s code does not change — `respond`, `catch_up`, `key`, `drain_ops` and
`drain_load_ops` are all called exactly as before, from the other process.

The same goes for `crates/neotmux/src/lib.rs`'s module doc ("M3 puts them in two processes —
the session where the jobs are, the plugins where the screen is") and `msg.rs`'s ("it puts the
**Lua layer on the client**"). Correct both to what the code does.

- [ ] **Step 11: Teach `socket.rs` the protocol as it is**

The raw-client helpers lose the round. `an_attach()` becomes
`neotmux_proto::msg::Attach { size: neotmux_proto::Size { rows: 24, cols: 80 } }`, and:

```rust
/// A raw client into the session: the greet, the attach, and the first frame it is owed.
fn raw_client(runtime: &TestRuntime) -> UnixStream {
    let mut stream = raw_attach(an_attach(), runtime);
    next_done(&mut stream);
    stream
}

/// The next frame on this link, whatever else the server says on the way to it.
fn next_done(stream: &mut UnixStream) -> neotmux_proto::msg::OwnedDone {
    loop {
        match read_message(stream) {
            OwnedToClient::Done(done) => return done,
            OwnedToClient::Clip(_) | OwnedToClient::Bye(_) => {}
            other => panic!("the server said {other:?} where a frame was owed"),
        }
    }
}
```

`frame_tick`, `frame_tock`, `frame_done` and `frame_once` go, and every call site becomes
`next_done`. `a_second_client_is_attached_side_by_side_with_the_first` asserts a `Done` rather
than a `Tick`.

Four tests are deleted, each for the state it was pinning no longer being split:

- `wants_is_the_or_of_the_clients_and_not_the_last_to_say_so` — there is one Lua layer, so
  there is no OR to take. (Spec §11.)
- `a_resized_handler_on_reattach_does_not_mint_a_window_id_that_already_exists` — one window
  id counter, which the reattaching client does not bring with it. (Spec §11.)
- `a_window_created_before_a_detach_and_one_created_after_do_not_share_an_id` — the same
  counter, for the same reason. (Spec §11.)
- `the_last_client_to_attach_is_the_one_the_session_is_told` — it pins the *bands* coming from
  the attach, and they come from the server's own Lua layer now. What it also said about the
  size is pinned by `a_session_is_refitted_to_the_terminal_that_attaches_to_it`, which stays.

`a_read_is_answered_on_the_link_that_asked_it_only` keeps its subject and loses its
choreography: with no tock to wait for, A asks and is answered, and B is never told.

```rust
#[test]
fn a_read_is_answered_on_the_link_that_asked_it_only() {
    let runtime = TestRuntime::new();
    let _session = started(runtime.path());

    let mut a = raw_client(&runtime);
    let mut b = raw_client(&runtime);

    send_message(
        &mut a,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::msg::ToServer::Read(
            neotmux_proto::msg::Read::Revision {
                pane: neotmux_proto::PaneId::from(1),
            },
        )),
    );
    let answered = loop {
        match read_message(&mut a) {
            neotmux_proto::OwnedToClient::Reading(reading) => break reading,
            // The frames the session keeps sending are not the answer, and not a failure
            // either: the answer comes on this link, after whatever was already on its way.
            _ => continue,
        }
    };
    assert!(
        matches!(answered, neotmux_proto::msg::Reading::Revision(Some(_))),
        "A's question is answered with the revision it asked for: {answered:?}"
    );

    // Whatever B is told from here on is the frames of a session that is alive. It must
    // never be a `Reading`: the question was asked on A's link and on no other.
    b.set_read_timeout(Some(Duration::from_millis(300)))
        .expect("b takes a patience");
    let mut len = [0u8; 4];
    use std::io::Read;
    for _ in 0..8 {
        let Ok(()) = b.read_exact(&mut len) else { break };
        let mut body = vec![0u8; usize::try_from(u32::from_le_bytes(len)).unwrap()];
        b.read_exact(&mut body).expect("b owes a body");
        let message = neotmux_proto::wire::decode_to_client(&body).expect("b's message decodes");
        assert!(
            !matches!(message, neotmux_proto::OwnedToClient::Reading(_)),
            "the link that did not ask was answered: {message:?}"
        );
    }
}
```

One test's prose is now wrong and its assertions are still right:
`a_reattached_client_builds_the_same_float_again` says "a float ... is the *client's* state: it
goes away with the client". It is the session's now, and it survives — the probe announces it
every tick either way, which is what the test actually watches. Rewrite the doc comment to say
that the float outlives the client and the announcement goes on; do not change what it asserts.

- [ ] **Step 12: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS. The end-to-end suites — `fixtures.rs`, `plugin_smoke.rs`, `terminal_restore.rs`,
`idle.rs`, `paste.rs`, `pty_flood.rs` — drive the real binary over a real PTY and are the net
under this whole task: they do not know where the Lua layer runs, and they must not start to.

If one of them fails, it is saying something true. Read it before changing it.

- [ ] **Step 13: Commit**

```bash
git add -A
git commit -m "the brain moves in beside the panes"
```

---

### Task 5: A session with nobody looking at it still ticks

This is the goal of the whole change, and until now the detached path has only drained panes.
`await_client` becomes a frame like any other, minus the clients.

**Files:**
- Modify: `crates/neotmux/src/server.rs` (`serve`, `await_client`, `frame`)
- Modify: `crates/neotmux/src/session.rs` (`forget_pane_output` goes)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: everything Task 4 produced.
- Produces: nothing new. `Session::forget_pane_output` is removed.

- [ ] **Step 1: Write the failing test**

In `socket.rs`:

```rust
/// What the whole move is for: a plugin goes on running when nobody is looking at it, and
/// it is the same plugin — one Lua layer for the session, not one per client, so its count
/// carries across the client that left.
#[test]
fn a_plugin_ticks_while_no_client_is_attached() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[(
        "init.lua",
        "local n = 0\n\
         neotmux.on_tick(function()\n\
         \x20 n = n + 1\n\
         \x20 if n % 10 == 0 then neotmux.log.info('ticked ' .. n) end\n\
         end)\n",
    )]);

    let mut first = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    wait_for_socket(&socket_in(runtime.path()));
    wait_for_log(runtime.path(), "ticked 10");

    first.kill();
    drop(first);

    // Nobody is attached for this second. The old server drained its panes and did nothing
    // else; this one is still ticking the plugin.
    let before = log_text(runtime.path()).matches("ticked ").count();
    std::thread::sleep(Duration::from_secs(1));
    let after = log_text(runtime.path()).matches("ticked ").count();
    assert!(
        after > before,
        "the plugin stopped when the client did: {before} lines before, {after} after\n{}",
        log_text(runtime.path())
    );

    // And it is the same one: a second layer would have started counting at one again.
    let second = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    wait_for_socket(&socket_in(runtime.path()));
    drop(second);
    let seen = log_text(runtime.path());
    assert_eq!(
        seen.matches("ticked 10\n").count(),
        1,
        "the count started again, so the session got a second Lua layer:\n{seen}"
    );
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p neotmux --test socket a_plugin_ticks_while_no_client_is_attached`
Expected: FAIL on the first assertion — the count does not move while the client is gone.

- [ ] **Step 3: Make the frame run without clients**

In `frame`, the early return on an empty client list is already only about `session` being
`None`; what has to change is that `serve` reaches `frame` at all with no clients. Replace
`await_client`'s body with the patience and the door, and let the frame do the rest:

```rust
/// A session with no client is kept for as long as its children go, and a server that never
/// got a client is not a session worth keeping. Both of those are decided here.
///
/// `true` is the session going on — with a client that has just come in, or with none, which
/// is no longer a reason to stop ticking. `false` is the server being over.
fn await_client(
    clients: &mut Vec<Attached>,
    pending: &mut Vec<Pending>,
    plugins: &mut Plugins,
    session: &mut Option<Session>,
    serving: &Serving<'_>,
    born: Instant,
) -> anyhow::Result<bool> {
    if serving.shutdown.raised().is_some() {
        return Ok(false);
    }
    // A session with a client that has left waits for ever: that is what detach means. A
    // server that never had one is not a session worth keeping.
    if session.is_none() {
        if Instant::now() >= born + STARTUP {
            return Ok(false);
        }
        door(pending, clients, plugins, session, serving)?;
        if session.is_none() {
            let until = (born + STARTUP).min(Instant::now() + FRAME);
            crate::tty::wait_on(std::os::fd::AsFd::as_fd(serving.listener), None, until)
                .context("wait for a client")?;
        }
    }
    Ok(true)
}
```

`serve`'s loop no longer guards on `clients.is_empty()`:

```rust
    loop {
        if shutdown.raised().is_some() {
            return Ok(());
        }
        if !await_client(&mut clients, &mut pending, &mut plugins, &mut session, &serving, born)? {
            return Ok(());
        }
        if frame(&mut session, &mut plugins, &clip, &mut clients, &mut pending, &serving)? {
            return Ok(());
        }
    }
```

The frame's own cadence is what paces a detached session: it ends with a wait that runs to
`FRAME` or until the panes nudge it, and with no client links in the set that wait is the
panes' alone. That is the cadence the old `await_client` kept too, so a detached session costs
what it already cost, plus the tick that is the point.

A session that runs out of shells while detached is `frame`'s `End::Eof`, which returns `true`
and ends the server — the same answer `session.idle()` used to give, now given by the path that
also drew the last frame.

- [ ] **Step 4: Delete `forget_pane_output`**

It exists because "nobody is subscribed to anything any more, because nobody is running any
plugins". They are now. Remove the method from `session.rs` and its two call sites in
`server.rs` (Task 4 removed one of them already); `Session::advance` sets `wants_pane_output`
from every tock, attached or not.

- [ ] **Step 5: Run the test, then everything**

Run: `cargo test -p neotmux --test socket a_plugin_ticks_while_no_client_is_attached`
Expected: PASS.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS. `idle.rs` is the suite that pins what a detached session does with its panes;
read its failures carefully if it has any.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux/src/server.rs crates/neotmux/src/session.rs crates/neotmux/tests/socket.rs
git commit -m "a session nobody is looking at is a session that is still running plugins"
```

---

### Task 6: The docs say where plugin code runs

**Files:**
- Modify: `docs/plugins.md`
- Modify: `docs/superpowers/specs/2026-09-23-lua-on-the-server-design.md` (§6 and §10, amended in place)

- [ ] **Step 1: `docs/plugins.md`**

Read it through and correct every place that says or implies the Lua layer is the client's.
At minimum:

- **Where your plugin runs.** A new short section near the top: plugin code runs in the
  server, one Lua layer per session, shared by every client attached to it. It keeps running
  while you are detached, which is what makes a watcher possible — and it is why a plugin that
  wedges costs the session rather than one terminal, which is what the budget is for.
- **`neotmux.clipboard.set`** (around line 688): it is answered by whichever clients are
  attached. With none attached it returns `false`, because there is no terminal to write the
  sequence to.
- **The exit transcript** (around line 714): it replays what was raised while *this* client was
  attached.
- **No blocking calls** (spec §4): state the rule in the plugin author's own terms — nothing on
  this API waits, and anything that has to wait is a subscription that delivers on a later
  tick. It is a contract, not an implementation note: a host call that blocks cannot be
  interrupted by the budget.

- [ ] **Step 2: Amend the spec**

Under §6, record that the protocol carries two more server-to-client messages than the table
says, and why (this plan's rulings 2 and 3). Under §10, mark stage 2 done and note that the
`Attach` shrink and the `PROTOCOL_VERSION` bump came with it rather than with stage 3, leaving
stage 3 the dead vocabulary. Amend rather than rewrite, the way §13 amends the earlier specs:
what was planned and what happened both stay readable.

- [ ] **Step 3: Commit**

```bash
git add docs/plugins.md docs/superpowers/specs/2026-09-23-lua-on-the-server-design.md
git commit -m "the docs say where a plugin runs"
```

---

## What this plan does not do

Stages 3 and 4 of spec §10, which each want their own plan:

- **The sweep.** `Tick`, `Tock`, `Step`, the epoch, the revisions, the pane info, `Read`,
  `Reading` and `reads.rs` are all unreferenced by the running program after Task 4 and are
  still on the wire. Removing them is mechanical and large, and doing it here would bury the
  move in it.
- **Two clients, corrected.** Shared copy mode and the rest of spec §7/§8 beyond the state
  already being shared.
- **The command surface.** Spec §9, whose shape this plan is careful not to foreclose: a
  connection that sends a command is not an `Attach`, and the doorstep already tells those
  apart.

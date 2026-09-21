# Detach and Attach Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A session outlives the terminal that was looking at it — `C-b d` leaves it running, so does a terminal that goes away, and running `neotmux` again shows the same panes with the same shells in them.

**Architecture:** Two changes carry the feature. The server's three-second `LINGER` goes, so a session ends only when its last shell does; and the attach path gains a second case, where a client arriving to an existing session has its `Attach` applied to that session rather than used to build one. Everything else hangs off those: one op, one end, one flag, one pid in a file.

**Tech Stack:** Rust 2024, `nix` (flock, signals, kill), `signal_hook`, `portable-pty` 0.9 in tests, no new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-21-detach-attach-design.md`

## Global Constraints

- `cargo test --workspace`, `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean at the end of every task. No `#[allow(...)]`.
- Tests pin behaviour, not structure. No speculative generality — two cases in the attach path, one op, one end, one flag.
- No host keybinding. `C-b d` is a line in `runtime/lua/plugin/prefix.lua`, exactly as every other pane and window action is.
- Comments only where the WHY is non-obvious, one line where possible. English throughout — code, comments, commit messages.
- No `Co-Authored-By` trailers on commits in this repo.
- `neotmux-core` keeps zero dependencies. This plan does not touch it.
- Named sessions, `ls`, `kill-session` and two clients on one session are **out of scope** (spec §1). The session name stays `default`.

---

### Task 1: A session that does not die when nobody is looking

**Files:**
- Modify: `crates/neotmux/src/server.rs` (the `LINGER` constant, and `give_up` inside `accept`)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Produces: nothing new in code. The behaviour later tasks rest on: with a session in hand, the server waits for a client without a deadline.

The server already keeps the session in an `Option<Session>` and already runs it while no client is attached, reading its children. All that changes is how long it is willing to do that.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux/tests/socket.rs`, below `the_server_outlives_a_client_that_disconnects`:

```rust
/// The old behaviour was a three-second linger. This is the same question asked long
/// enough after the client is gone that only an immortal session can answer it.
#[test]
fn a_session_with_nobody_looking_at_it_outlives_the_old_linger() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let mut client = started(runtime.path());

    client.kill();
    drop(client);
    std::thread::sleep(Duration::from_secs(5));

    let mut stream = UnixStream::connect(socket_in(runtime.path()))
        .expect("the session died while nobody was looking at it");
    let mut hello = [0u8; HELLO_LEN];
    stream.write_all(&wire::encode_hello()).expect("greet");
    stream.read_exact(&mut hello).expect("the server did not greet");
    assert_eq!(hello, wire::encode_hello());
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --test socket a_session_with_nobody_looking`
Expected: FAIL at `the session died while nobody was looking at it` — the connect gets `ECONNREFUSED` once the three seconds are up.

- [ ] **Step 3: Remove the linger**

In `crates/neotmux/src/server.rs`, delete the `LINGER` constant and its doc comment, and change `give_up` in `accept` so a session waits without a deadline:

```rust
    // A session with a client to wait for waits for ever: that is what detach means. A
    // server that never had one is not a session worth keeping, so `STARTUP` still applies.
    let give_up = match session {
        Some(_) => None,
        None => Some(born + STARTUP),
    };
    loop {
        if shutdown.raised().is_some() || give_up.is_some_and(|at| Instant::now() >= at) {
            return Ok(None);
        }
        let frame_end = Instant::now() + FRAME;
        let ready = crate::tty::wait_on(
            std::os::fd::AsFd::as_fd(listener),
            session.is_some().then(|| wakeup.as_fd()),
            give_up.map_or(frame_end, |at| at.min(frame_end)),
        )
        .context("wait for a client")?;
```

The rest of `accept` — clearing the wakeup, `session.idle()?`, taking the stream — is unchanged.

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p neotmux --test socket`
Expected: PASS, including the existing `the_server_outlives_a_client_that_disconnects`.

- [ ] **Step 5: Update the doc comment on `serve`**

`crates/neotmux/src/server.rs` opens with a module comment saying the session is kept for `LINGER` and that "Nothing attaches twice yet — that is detach/attach, the next milestone". Rewrite those two sentences to say what is now true: the session is kept until its last shell exits, and a second client is still refused while one is attached.

- [ ] **Step 6: Run the whole suite**

Run: `cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux/src/server.rs crates/neotmux/tests/socket.rs
git commit -m "server: a session that ends when its last shell does, and not before"
```

---

### Task 2: Attaching to a session that is already there

**Files:**
- Modify: `crates/neotmux/src/session.rs` (add `Session::adopt`)
- Modify: `crates/neotmux/src/server.rs` (`attached`, and its call site in `serve`)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: Task 1's immortal session.
- Produces: `Session::adopt(&mut self, attach: &Attach) -> anyhow::Result<()>` — applies a new client's `Attach` to an existing session and leaves it ready to be shown to that client. `server::attached` becomes `fn attached(link: &mut Link, waker: &Waker, session: Option<Session>) -> anyhow::Result<Option<Session>>`.

`Attach` does not change shape. It is read twice over: as the arguments a session is built from, and as values applied to one that exists.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux/tests/socket.rs`:

```rust
/// The shell is `/bin/cat`, so what was typed before the client went away is still on the
/// pane's grid afterwards — and only a kept terminal state can show it again.
#[test]
fn a_second_client_is_shown_the_session_the_first_one_left() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let mut first = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    first.send("still-here-after-the-detach\r");
    first.wait_for_output("still-here-after-the-detach");
    first.kill();
    drop(first);

    let second = Client::start(runtime.path(), "/bin/cat");
    second.wait_for_output("still-here-after-the-detach");
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --test socket a_second_client_is_shown`
Expected: FAIL — the second client is turned away with "this session already has a client attached", because `serve` refuses any stream while `session.is_some()`.

- [ ] **Step 3: Write `Session::adopt`**

In `crates/neotmux/src/session.rs`, next to `resize`:

```rust
    /// A new client taking over a session that is already running. `Attach`'s fields are
    /// values to apply here rather than arguments to build from: the terminal is this
    /// client's, and so are the bands, the gutter and the subscriptions, because the Lua
    /// layer that declared them is new.
    ///
    /// `first_window` is ignored — the windows already have ids.
    pub fn adopt(&mut self, attach: &Attach) -> anyhow::Result<()> {
        self.surface.size = clamp_to_min_grid(attach.size);
        self.surface.reserved = attach.reserved;
        self.surface.gutter = attach.gutter;
        self.wants_pane_output = attach.wants_pane_output;
        // What happened while nobody was looking is not news for a client that was not
        // there: geometry is how a plugin learns what exists, events how it learns what
        // changed, and everything here is the former.
        self.steps.clear();
        self.apply_geometry()?;
        self.publish_geometry();
        // Nothing this client has been shown is on its screen, so the first frame is not
        // allowed to be "nothing changed".
        self.last_overlay.clear();
        self.last_notices.clear();
        self.dirty = true;
        Ok(())
    }
```

- [ ] **Step 4: Branch the server's attach path**

In `crates/neotmux/src/server.rs`, replace `attached` with:

```rust
/// The client's first message, applied. With a session in hand this adopts it; without one
/// it builds it. `None` means the client went away before it said anything.
fn attached(
    link: &mut Link,
    waker: &Waker,
    session: Option<Session>,
) -> anyhow::Result<Option<Session>> {
    let Some(message) = link.recv().context("read the client's first message")? else {
        return Ok(None);
    };
    let attach = match wire::decode_to_server(&message).context("decode the attach")? {
        ToServer::Attach(attach) => attach,
        other => anyhow::bail!("a client's first message was {other:?} rather than an attach"),
    };
    Ok(Some(match session {
        Some(mut session) => {
            session.adopt(&attach)?;
            session
        }
        None => Session::new(&attach, waker.clone())?,
    }))
}
```

And in `serve`, delete the `if session.is_some() { turn_away(stream, BUSY); continue; }` arm — a client arriving when no client is attached is what this feature is — and hand the session to `attached`:

```rust
        let mut running = match attached(&mut link, &waker, session.take())? {
            Some(started) => started,
            None => continue,
        };
```

`session` is already `Option<Session>`; `take()` leaves it `None` while the conversation runs, which is what the `BUSY` arm inside `converse` keys off — that one stays, and is what still refuses a *second* client while one is attached.

- [ ] **Step 5: Run it and watch it pass**

Run: `cargo test -p neotmux --test socket`
Expected: PASS, including `a_second_client_is_refused_and_says_why` — that test starts its second client while the first is still attached, which `converse` still turns away.

- [ ] **Step 6: Write the resize test**

`Client::start` hard-codes 24x80. Add a sized constructor beside it in `crates/neotmux/tests/socket.rs`:

```rust
impl Client {
    fn start(runtime: &Path, shell: &str) -> Self {
        Self::start_sized(runtime, shell, 24, 80)
    }

    fn start_sized(runtime: &Path, shell: &str, rows: u16, cols: u16) -> Self {
        // the existing body, with `rows` and `cols` in the PtySize
    }
}
```

and the test:

```rust
/// The session is laid out against whichever terminal is looking at it now.
#[test]
fn a_session_is_refitted_to_the_terminal_that_attaches_to_it() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let first = Client::start_sized(runtime.path(), "/bin/sh", 24, 80);
    wait_for_socket(&socket_in(runtime.path()));
    drop(first);

    let mut second = Client::start_sized(runtime.path(), "/bin/sh", 30, 100);
    second.send("tput cols\r");
    second.wait_for_output("100");
}
```

- [ ] **Step 7: Run it**

Run: `cargo test -p neotmux --test socket a_session_is_refitted`
Expected: PASS — `adopt` calls `apply_geometry`, which resizes every showable pane's PTY, and the shell asks that PTY how wide it is.

- [ ] **Step 8: Run the whole suite**

Run: `cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add crates/neotmux/src/session.rs crates/neotmux/src/server.rs crates/neotmux/tests/socket.rs
git commit -m "attach: a client that arrives to a session is shown it rather than given a new one"
```

---

### Task 3: Window ids that do not collide with the ones already there

**Files:**
- Modify: `crates/neotmux-lua/src/window.rs` (`WindowIds::seen`)
- Modify: `crates/neotmux-lua/src/lib.rs` (`republish_geometry`)
- Test: in `crates/neotmux-lua/src/window.rs`, and `crates/neotmux-lua/tests/attach.rs`

**Interfaces:**
- Consumes: Task 2's adopted session, which publishes its geometry to the new client.
- Produces: `WindowIds::seen(&self, id: WindowId)` — records an id handed out by somebody else, so the next `take()` is past it.

`window.create()` answers with an id before the op that makes the window is applied (§4.6), which is why the counter is on the client and cannot move to the server. A client attaching to an existing session therefore has to learn the watermark, and geometry is where it is written down.

- [ ] **Step 1: Write the failing unit test**

In `crates/neotmux-lua/src/window.rs`'s test module:

```rust
    #[test]
    fn an_id_somebody_else_handed_out_is_never_handed_out_again() {
        let ids = WindowIds::new();
        ids.seen(WindowId::from(7));
        assert_eq!(ids.take(), WindowId::from(8));
        // A lower one changes nothing: the counter only ever moves forwards.
        ids.seen(WindowId::from(2));
        assert_eq!(ids.take(), WindowId::from(9));
    }
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux-lua window`
Expected: FAIL — no method named `seen`.

- [ ] **Step 3: Implement it**

```rust
    /// An id this counter did not hand out — a window that was already there when this
    /// client attached. Ids are never reused, so the counter moves past it and never back.
    pub fn seen(&self, id: WindowId) {
        self.next.set(self.next.get().max(id.get() + 1));
    }
```

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p neotmux-lua window`
Expected: PASS.

- [ ] **Step 5: Write the failing test for the wiring**

Create `crates/neotmux-lua/tests/attach.rs`. Follow the harness the other files in that directory use for building a `LuaSubsystem`; the assertion is:

```rust
/// A client attaching to a running session is told what is there by geometry, and takes its
/// window-id watermark from it — otherwise `window.create()` would name a window that exists.
#[test]
fn a_published_geometry_moves_the_window_id_counter_past_what_it_names() {
    let lua = subsystem();
    lua.republish_geometry(Geometry {
        size: Size { rows: 24, cols: 80 },
        panes: vec![(PaneId::from(1), Rect { x: 0, y: 0, w: 80, h: 24 })],
        focus: Some(PaneId::from(1)),
        windows: vec![Window {
            id: WindowId::from(4),
            panes: vec![PaneId::from(1)],
        }],
        window: Some(WindowId::from(4)),
    });
    assert_eq!(lua.reserve_window_id(), WindowId::from(5));
}
```

Check `crates/neotmux-proto/src/geometry.rs` for `Window`'s exact fields before writing this, and match them.

- [ ] **Step 6: Run it and watch it fail**

Run: `cargo test -p neotmux-lua --test attach`
Expected: FAIL — the counter still answers 1.

- [ ] **Step 7: Wire it up**

In `crates/neotmux-lua/src/lib.rs`, in `republish_geometry`, before storing the geometry:

```rust
        // Ids the host is naming that this counter did not hand out: a client attaching to a
        // session that was already running learns the watermark here and nowhere else.
        for window in &geometry.windows {
            self.window_ids.seen(window.id);
        }
```

- [ ] **Step 8: Run it and watch it pass**

Run: `cargo test -p neotmux-lua --test attach`
Expected: PASS.

- [ ] **Step 9: Run the whole suite**

Run: `cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 10: Commit**

```bash
git add crates/neotmux-lua/src/window.rs crates/neotmux-lua/src/lib.rs crates/neotmux-lua/tests/attach.rs
git commit -m "lua: a window id counter that starts past the windows already there"
```

---

### Task 4: `neotmux.detach()`, and `C-b d`

**Files:**
- Modify: `crates/neotmux-proto/src/lib.rs` (`PROTOCOL_VERSION`)
- Modify: `crates/neotmux-proto/src/msg.rs` (`SessionOp::Detach`, `End::Detach`)
- Modify: `crates/neotmux-proto/src/wire.rs` (`OP_DETACH`, `END_DETACH`, both directions, and the round-trip tests)
- Modify: `crates/neotmux-lua/src/ops.rs` (`Op::Detach`)
- Modify: `crates/neotmux-lua/src/api/mod.rs` (`neotmux.detach`)
- Modify: `crates/neotmux/src/plugins.rs` (`Op::Detach` → `SessionOp::Detach`, and `drain_load_ops`)
- Modify: `crates/neotmux/src/session.rs` (`apply_ops` returns what ended, not a bool)
- Modify: `crates/neotmux/src/server.rs` (`End::Detach` keeps the session)
- Modify: `runtime/lua/plugin/prefix.lua`, `docs/plugins.md`
- Test: `crates/neotmux-proto/src/wire.rs`, `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: Task 2's adopt path — a detached session is one that gets adopted next time.
- Produces: `SessionOp::Detach`, `End::Detach`, `Op::Detach`, and the Lua call `neotmux.detach()`.

`neotmux.quit()`'s whole road is reused: an op raised from Lua, applied after the tick, answered with an end the client acts on. The one difference is what the server does after answering.

- [ ] **Step 1: Write the failing wire test**

In `crates/neotmux-proto/src/wire.rs`'s test module, extend the two existing round-trip tests rather than adding new ones: the `Tock` ops list (near the `SessionOp::Quit` case around line 2070) gains

```rust
                ("plugin/prefix".to_string(), SessionOp::Detach),
```

and the `End` round trip (around line 2152) becomes

```rust
        for end in [None, Some(End::Quit), Some(End::Eof), Some(End::Detach)] {
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux-proto`
Expected: FAIL to compile — no variant `Detach`.

- [ ] **Step 3: Add the variants and their codes**

- `crates/neotmux-proto/src/msg.rs`: `SessionOp::Detach` and `End::Detach`, each with a one-line doc saying the session is kept and only the client leaves.
- `crates/neotmux-proto/src/wire.rs`: `const OP_DETACH: u8 = 6;` and `const END_DETACH: u8 = 2;`, with both the encode arms and the decode arms.
- `crates/neotmux-proto/src/lib.rs`: `pub const PROTOCOL_VERSION: u32 = 3;`

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p neotmux-proto`
Expected: PASS.

- [ ] **Step 5: Give Lua the call**

`crates/neotmux-lua/src/ops.rs`: add `Detach` to `Op`.

`crates/neotmux-lua/src/api/mod.rs`, in the block beside `quit`:

```rust
    {
        let sources = sources.clone();
        let ops = ops.clone();
        let detach = lua.create_function(move |lua, ()| {
            ops.push(source::caller(lua, &sources), Op::Detach);
            Ok(())
        })?;
        neotmux.set("detach", detach)?;
    }
```

`crates/neotmux/src/plugins.rs`: `Op::Detach => SessionOp::Detach` beside the `Quit` arm, and in `drain_load_ops` a `SessionOp::Detach` arm that refuses it the way the others are refused, with the reason `"there is no session to detach from until it starts"`.

- [ ] **Step 6: Make the session say which end it was**

In `crates/neotmux/src/session.rs`, `apply_ops` currently returns `anyhow::Result<bool>` meaning "quit". Change it to `anyhow::Result<Option<End>>`: `SessionOp::Quit => ended = Some(End::Quit)`, `SessionOp::Detach => ended = Some(End::Detach)`, and let a later op not overwrite an earlier end. In `advance`, the early return becomes

```rust
        if let Some(end) = self.apply_ops(&tock.ops)? {
            return Ok(Pass {
                steps: std::mem::take(&mut self.steps),
                frame: None,
                end: Some(end),
            });
        }
```

- [ ] **Step 7: Make the server keep the session on a detach**

In `crates/neotmux/src/server.rs`'s `converse`, the arm that ends the process on `end.is_some()` must now distinguish the two:

```rust
        // Quit and Eof are the session ending, and the session is all this process is for.
        // A detach is the client ending: the session stays, and the loop goes back to
        // waiting for somebody to look at it.
        match end {
            Some(End::Quit) | Some(End::Eof) => return Ok(true),
            Some(End::Detach) => return Ok(false),
            None => {}
        }
```

`server.rs` imports `neotmux_proto::msg::{Done, ToClient, ToServer}`; add `End` to that list.

`converse` returning `false` already means "the client went, the session did not", which is the path that calls `forget_pane_output` and goes back to `accept`. The client side needs nothing new: `tui::run` already leaves its loop on `done.end.is_some()` and returns `Ok(None)`, which `main` turns into a clean exit.

- [ ] **Step 8: Write the end-to-end test**

In `crates/neotmux/tests/socket.rs`:

```rust
/// `C-b d`: the client exits cleanly, the session does not, and what was in the pane is
/// still there when something attaches to it again.
#[test]
fn a_detach_leaves_the_client_and_keeps_the_session() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let mut first = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    first.send("survives-a-detach\r");
    first.wait_for_output("survives-a-detach");
    first.send("\x02d");
    assert!(
        first.wait_for_exit().success(),
        "a detached client did not exit cleanly"
    );

    let second = Client::start(runtime.path(), "/bin/cat");
    second.wait_for_output("survives-a-detach");
}
```

- [ ] **Step 9: Run it and watch it fail**

Run: `cargo test -p neotmux --test socket a_detach_leaves_the_client`
Expected: FAIL — `C-b d` is not bound, so the client sits there and `wait_for_exit` times out.

- [ ] **Step 10: Bind the key**

In `runtime/lua/plugin/prefix.lua`, in the `commands` table:

```lua
    ["d"] = function()
      neotmux.detach()
    end,
```

and add `d` to the key list in the file's header comment.

- [ ] **Step 11: Run it and watch it pass**

Run: `cargo test -p neotmux --test socket`
Expected: PASS.

- [ ] **Step 12: Document the call**

In `docs/plugins.md`, in the "Everything else" block beside `neotmux.quit()`:

```lua
neotmux.detach()          -- leave the session running and let go of this terminal
```

and a short paragraph under it saying what a detach does not carry: the panes, the shells and each pane's scrollback are the server's and stay; every plugin's own state is the client's and does not — the copy register, a raised mode, a fold, and the reservations, which the plugin re-makes at load. Say that re-attaching is a plugin restart, in those words.

- [ ] **Step 13: Run the whole suite**

Run: `cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 14: Commit**

```bash
git add crates/neotmux-proto crates/neotmux-lua crates/neotmux/src crates/neotmux/tests runtime/lua/plugin/prefix.lua docs/plugins.md
git commit -m "detach: one op, one end, and a session that stays behind"
```

---

### Task 5: `neotmux --kill`

**Files:**
- Modify: `crates/neotmux/src/sock.rs` (the pid in the lock file, and reading it back)
- Modify: `crates/neotmux/src/main.rs` (`--kill`)
- Test: in `crates/neotmux/src/sock.rs`, `crates/neotmux/src/main.rs`, and `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: Task 1's immortal session — the thing there now needs a way to be ended from outside.
- Produces: `Lock::take(&Paths) -> anyhow::Result<Option<Lock>>` (unchanged signature; it now writes the pid) and `sock::holder(&Paths) -> anyhow::Result<Option<i32>>` — the pid of the process holding this session's lock, or `None` when nobody holds it.

Deliberately not a protocol message: the case that most needs `--kill` is a rebuilt binary meeting a server that refuses its handshake (§6). A pid in a file is a hint, so the lock is what decides whether anything is signalled.

- [ ] **Step 1: Write the failing test for the lock's pid**

In `crates/neotmux/src/sock.rs`'s test module:

```rust
    #[test]
    fn the_lock_holder_is_named_while_it_is_held_and_nobody_after() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            socket: dir.path().join("default.sock"),
            lock: dir.path().join("default.lock"),
        };
        assert_eq!(holder(&paths).expect("holder"), None, "nobody holds it yet");

        let held = Lock::take(&paths).expect("take").expect("nobody held it");
        assert_eq!(
            holder(&paths).expect("holder"),
            Some(std::process::id() as i32)
        );

        drop(held);
        assert_eq!(holder(&paths).expect("holder"), None, "it was given back");
    }
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --lib sock`
Expected: FAIL to compile — no function `holder`.

- [ ] **Step 3: Write the pid, and read it back**

In `Lock::take`, after the lock is acquired, truncate the file and write the pid followed by a newline. In the same module:

```rust
/// The pid of the process serving this session, or `None` when nobody is.
///
/// The lock is what answers "is anybody there" — a lock this process can take is a lock
/// nobody holds, whatever a file left behind says. The pid inside it is only the name of
/// whoever holds it, and is never read without the lock having refused first.
pub fn holder(paths: &Paths) -> anyhow::Result<Option<i32>> {
    if Lock::take(paths)?.is_some() {
        return Ok(None);
    }
    let text = fs::read_to_string(&paths.lock)
        .with_context(|| format!("read {}", paths.lock.display()))?;
    Ok(text.trim().parse().ok())
}
```

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p neotmux --lib sock`
Expected: PASS.

- [ ] **Step 5: Write the failing test for the flag**

In `crates/neotmux/src/main.rs`'s test module, beside the existing `parse_args` tests:

```rust
    #[test]
    fn the_kill_flag_is_recognised_and_is_not_a_warning() {
        let flag = parse_args(args(&["--kill"]));
        assert!(flag.kill);
        assert!(!flag.server);
        assert!(flag.warnings.is_empty());
    }
```

- [ ] **Step 6: Run it and watch it fail**

Run: `cargo test -p neotmux --bin neotmux parse`
Expected: FAIL to compile — `Cli` has no field `kill`.

- [ ] **Step 7: Add the flag and what it does**

`Cli` gains `kill: bool`, `parse_args` gains the `"--kill" => kill = true` arm, and `main` dispatches it before the client:

```rust
    if cli.kill {
        return kill();
    }
```

with, in the same file:

```rust
/// Ends the session's server from outside, for the case the protocol cannot reach: a client
/// built against another version is refused by the handshake, and the session it cannot
/// speak to would otherwise be unreachable and immortal.
fn kill() -> anyhow::Result<()> {
    let paths = neotmux::sock::Paths::resolve(&|key| env::var_os(key))?;
    let Some(pid) = neotmux::sock::holder(&paths)? else {
        anyhow::bail!("no neotmux session is running");
    };
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .with_context(|| format!("signal the neotmux server (pid {pid})"))?;
    println!("neotmux: asked the session (pid {pid}) to end");
    Ok(())
}
```

Check that `anyhow::Context` is in scope in `main.rs`; add the `use` if it is not.

- [ ] **Step 8: Run it and watch it pass**

Run: `cargo test -p neotmux --bin neotmux`
Expected: PASS.

- [ ] **Step 9: Write the end-to-end tests**

In `crates/neotmux/tests/socket.rs`:

```rust
/// The way out of an immortal session, and the one that does not go through the protocol.
#[test]
fn kill_ends_a_session_nobody_is_attached_to() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let mut client = started(runtime.path());
    client.kill();
    drop(client);

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("--kill")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .status()
        .expect("run --kill");
    assert!(status.success());

    let socket = socket_in(runtime.path());
    let deadline = Instant::now() + PATIENCE;
    while UnixStream::connect(&socket).is_ok() {
        assert!(Instant::now() < deadline, "the session outlived --kill");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn kill_with_nothing_running_says_so_and_fails() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("--kill")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run --kill");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("no neotmux session is running"), "{said}");
}
```

- [ ] **Step 10: Run them**

Run: `cargo test -p neotmux --test socket kill`
Expected: PASS.

- [ ] **Step 11: Run the whole suite**

Run: `cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 12: Commit**

```bash
git add crates/neotmux/src/sock.rs crates/neotmux/src/main.rs crates/neotmux/tests/socket.rs
git commit -m "kill: a way out that does not go through the protocol"
```

---

### Task 7: `--help` and `--version`

**Files:**
- Modify: `crates/neotmux/src/main.rs` (`Cli`, `parse_args`, `main`, and its test module)

**Interfaces:**
- Consumes: Task 5's `--kill`, which is the flag that makes the CLI worth describing.
- Produces: nothing other tasks depend on. Task 6 documents what this prints.

Until now the command line was `--server` and `--log`, both of them things a user never types — one is how the binary starts its own other half, the other is for debugging. `--kill` is the first flag a person is meant to reach for, which is the moment a program owes them a way to ask what the flags are. Read `crates/neotmux/src/main.rs` as Task 5 left it rather than as this task imagines it.

**The help text covers the command line and nothing else.** No keybindings: `C-b` and every key under it belong to `runtime/lua/plugin/prefix.lua` and can be rebound or removed in a line of config, so a help text that lists them is lying to anyone who did. `docs/plugins.md` and the README own that.

- [ ] **Step 1: Write the failing tests**

In `crates/neotmux/src/main.rs`'s test module, beside the existing `parse_args` tests:

```rust
    #[test]
    fn the_help_and_version_flags_are_recognised_in_both_spellings() {
        for spelling in ["--help", "-h"] {
            let flag = parse_args(args(&[spelling]));
            assert!(flag.help, "{spelling}");
            assert!(flag.warnings.is_empty(), "{spelling}");
        }
        for spelling in ["--version", "-V"] {
            let flag = parse_args(args(&[spelling]));
            assert!(flag.version, "{spelling}");
            assert!(flag.warnings.is_empty(), "{spelling}");
        }
    }

    /// An unrecognised argument is still a warning rather than a refusal — but now there is
    /// somewhere to send the person who typed it.
    #[test]
    fn an_unrecognised_argument_points_at_the_help() {
        let flag = parse_args(args(&["--wat"]));
        assert_eq!(flag.warnings.len(), 1);
        assert!(flag.warnings[0].contains("--wat"), "{:?}", flag.warnings[0]);
        assert!(
            flag.warnings[0].contains("neotmux --help"),
            "{:?}",
            flag.warnings[0]
        );
    }

    /// The version says both numbers: a client and a server from different commits refuse
    /// each other by protocol number, and this is what the person reads next.
    #[test]
    fn the_version_line_carries_the_protocol_number_too() {
        let said = version_line();
        assert!(said.contains(env!("CARGO_PKG_VERSION")), "{said}");
        assert!(
            said.contains(&neotmux_proto::PROTOCOL_VERSION.to_string()),
            "{said}"
        );
    }
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux --bin neotmux`
Expected: FAIL to compile — `Cli` has no `help` or `version` field and there is no `version_line`.

- [ ] **Step 3: Implement**

`Cli` gains `help: bool` and `version: bool`. `parse_args` gains the four spellings, and its unrecognised-argument arm gains the pointer:

```rust
            "--help" | "-h" => help = true,
            "--version" | "-V" => version = true,
            _ => warnings.push(format!(
                "unrecognised argument '{arg}'; ignoring it — try 'neotmux --help'"
            )),
```

and beside them:

```rust
/// Both numbers, because they fail in different ways: the first is what was installed, the
/// second is what a client and a server have to agree on before they will talk at all.
fn version_line() -> String {
    format!(
        "neotmux {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        neotmux_proto::PROTOCOL_VERSION
    )
}

/// What the command line offers. The keys are not here: every one of them belongs to a
/// plugin that can drop or rebind it, so `docs/plugins.md` is where they are written down.
fn usage() -> String {
    format!(
        "\
{}

usage: neotmux [options]

With no options, neotmux attaches to this session if it is running and starts it if it
is not. C-b d leaves a session running; neotmux on its own brings you back to it.

options:
  --kill          end the running session and every shell in it
  --log <path>    write the log somewhere other than the default
  --help, -h      this text
  --version, -V   the version, and the protocol number it speaks
  --server        run the half that holds the session; started for you, not by you
",
        version_line()
    )
}
```

In `main`, before anything that touches the terminal or the socket — and before the `--kill` arm, so that `neotmux --help --kill` explains itself rather than killing a session:

```rust
    if cli.help {
        print!("{}", usage());
        return Ok(());
    }
    if cli.version {
        println!("{}", version_line());
        return Ok(());
    }
```

- [ ] **Step 4: Run them and watch them pass**

Run: `cargo test -p neotmux --bin neotmux`
Expected: PASS, the pre-existing `parse_args` tests included.

- [ ] **Step 5: See it with your own eyes**

Run: `cargo run -p neotmux -- --help` and `cargo run -p neotmux -- --version`.
Read what they print. A help text is read by people, so look at it as one — the alignment, the wrapping at 80 columns, and whether the first line answers "what is this". Fix what reads badly; that is the deliverable here, not the flags.

- [ ] **Step 6: Run the whole suite**

Run: `cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux/src/main.rs
git commit -m "cli: a way to ask what the flags are, and which protocol this one speaks"
```

---

### Task 6: Saying so — README, CHANGELOG, and the spec's roadmap

**Files:**
- Modify: `README.md` (the Scope section, around line 38)
- Modify: `CHANGELOG.md` (the Unreleased section)
- Modify: `docs/superpowers/specs/2026-09-19-neotmux-design.md` (§11's M3 line)

**Interfaces:**
- Consumes: everything above. Nothing consumes this.

The README currently tells a reader that detach/attach is not in the product. Leaving that standing after shipping it is the same class of error as a stale doc comment, and it is the first thing anyone reads.

- [ ] **Step 1: Fix the README**

Replace the paragraph beginning "v0.1 deliberately does **not** include detach/attach" with what is now true: detach and attach are in, `C-b d` leaves a session running, running `neotmux` again attaches to it, `neotmux --kill` ends one from outside, and a re-attach reloads the plugins — the panes and shells are the server's and stay, the plugin state is the client's and does not. Keep the sentence about multiple sessions being out.

- [ ] **Step 2: Write the CHANGELOG entry**

Add a section to `Unreleased` in the voice of the entries already there — what a user would notice first, then the two things worth knowing. Cover: the session outliving the terminal on purpose and by accident; `C-b d`; `neotmux` attaching to what is there; `--kill` and why it is not a protocol message; `PROTOCOL_VERSION` 3, so a client and server from different commits refuse each other; and plainly, that re-attaching restarts the plugins.

- [ ] **Step 3: Correct the spec's roadmap line**

In `docs/superpowers/specs/2026-09-19-neotmux-design.md` §11, the M3 bullet says "server/client split, detach/attach" as a future. Mark it done and point at `docs/superpowers/specs/2026-09-21-detach-attach-design.md`, so the older spec does not read as the current plan.

- [ ] **Step 4: Verify every claim you just wrote**

Run: `make install PREFIX=$(mktemp -d)` and drive the installed binary by hand — start it, type something, `C-b d`, start it again and see what you typed, then `--kill`. Nothing in the README or the CHANGELOG may be claimed without having watched it happen. Use `cargo test --workspace` for the rest.

- [ ] **Step 5: Commit**

```bash
git add README.md CHANGELOG.md docs/superpowers/specs/2026-09-19-neotmux-design.md
git commit -m "docs: detach and attach, said where a reader will find it"
```

---

## Self-review notes

Spec coverage, section by section:

- §3 the session's lifetime → Task 1.
- §4 attaching to a session already there, including the window-id watermark → Tasks 2 and 3.
- §5 what a detach does not carry → documented in Task 4 step 12 and Task 6 step 2. There is no code for it: it is what happens when a process ends.
- §6 `neotmux.detach()`, the protocol bump, the prefix key → Task 4. `--kill` and the pid in the lock → Task 5.
- §7 error handling: a client dying mid-frame → Task 2's test plus the existing `the_server_outlives_a_client_that_disconnects`; a second client → the existing `a_second_client_is_refused_and_says_why`, kept passing in Task 2 step 5; a version mismatch → the existing `a_client_meeting_another_protocol_says_both_numbers_and_exits`; a stale pid → Task 5 step 3, where the lock decides.
- `--help` / `--version` → Task 7, added after the plan was written: the CLI grew a
  flag a person is meant to type, so it owes them a way to ask what the flags are.
- §8 tests → one per task, named in the task that makes the behaviour true. The "status line after a re-attach" check in the spec's list is covered by Task 2's test, which asserts the session's content comes back rather than the bar's text; if that feels thin at review time, add it to Task 2 rather than carrying a task for it.

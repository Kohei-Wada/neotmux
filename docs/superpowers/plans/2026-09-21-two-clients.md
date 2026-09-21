# Two Clients on One Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:subagent-driven-development's sibling superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two terminals attach to the same session; a key typed on either reaches the same focused pane, each terminal keeps its own plugin state, and the frame loop — today a conversation with one client — becomes a frame with all of them.

**Architecture:** The wire protocol is unchanged (spec §7 says no message changes shape, so `PROTOCOL_VERSION` does not move); what changes is the server. `Session::advance` is split into the part a tock does to a session (`apply`) and the part it makes of a picture (`frame_with`), because with two clients the overlay and notices a frame carries are each client's own. `server.rs` is rewritten from `accept → attached → converse` into `await_client` (nobody looking) plus `frame` (everybody attached), over a new N-fd `tty::wait_any`.

**Tech Stack:** Rust 2024, `nix::poll` (already a workspace feature, `tty.rs` already polls two fds this way), `anyhow`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-21-sessions-design.md` — §6–§7 are this plan, §8 the error rules, §10 the tests.

**Depends on:** the `2026-09-21-named-sessions.md` plan having landed — for order, and for the two `tests/socket.rs` `kill` invocations its Task 2 re-spelt (this plan's flipped `a_second_client_is_attached_side_by_side_with_the_first` lives next to them — not for test plumbing, because this plan's tests are in `tests/socket.rs` and stand on its `TestRuntime` and `Client`, the ones at the line `started()` shows, and do not need the harness at all.

## Global Constraints

- **Protocol unchanged.** No message gains, loses or reshapes a field; no `PROTOCOL_VERSION` bump. `ToClient::Refused` stays in the protocol for the cases that still refuse; the `BUSY` the second client used to get is the one thing it stops getting.
- **Shared input, fan-out output.** A `ToServer::Key` from any attached client reaches the focused pane. The tick and the done of each frame reach every attached client. (spec §6)
- **Geometry is the session's.** `size` is last `Resize` wins; `reserved`/`gutter` is the last client to be adopted (its tock applies last, in attach order). A client never gets the other client's bands merged in. (spec §6)
- **Failure isolation.** Nothing one client does to its own connection ends the session or costs another client anything: an undecodable message, a hangup, a slow tick — the rule `attached` already kept, now per link. (spec §7)
- **`wants_pane_output` is the OR** of what the attached clients asked, recomputed whenever the set changes. (spec §6)
- A frame waits for every attached client's tock; that is accepted, documented slowness, not a timeout. A client that stops answering is a client that has gone, at the point its connection stops.
- Lua-layer changes: none. This plan adds no plugin function; `neotmux.session_name()` (the other plan) is the only new ABI in the pair.

## Review Focus

- A client dies mid-frame — after its tick, before or while its tock is coming. The frame still completes for the rest, and the session does not.
- Both clients type, into the same focused pane, in whatever order the sockets deliver. What the *other* client saw must not reach its own plugins' input path — the line that is already the server's is `Key`, and only `Key`.
- One client with a *different* terminal size resizes the session mid-frame; the other client's next frame is the other client's terminal drawing it, and neither has to be told why.
- A client whose plugins read a pane (a `Read` mid-tick) is answered on arrival, on its own link, with the other client's in-flight tick not blocking that answer and its answer not leaking to the other link.
- `kill work` with two clients attached kills both connections and the children, exactly as it kills the one connection today — the tests that say so are the two that catch a regression in either direction.
- A `Tock` sent to a client the frame is no longer inside of (arrived late after a detach) is ignored, as it is today, rather than applied as a ghost.

---

### Task 1: One wait, any number of things

**Files:**
- Modify: `crates/neotmux/src/tty.rs` (next to `wait_on`; its `mod tests`)

**Interfaces:**
- Consumes: `nix::poll` (already used by `wait_on` in this file).
- Produces: `tty::wait_any(slots: &[std::os::fd::BorrowedFd], deadline: std::time::Instant) -> io::Result<Any>`, `pub enum Any { Ready(usize), HangUp(usize), Idle }`, where the index is the slot's position in `slots`.

- [ ] **Step 1: Write the failing tests**

In `crates/neotmux/src/tty.rs`, in the existing `mod tests` (next to `a_quiet_terminal_is_idle_until_the_deadline`; it already has a `pty_pair()` helper this test reuses):

```rust
    #[test]
    fn the_quiet_slots_are_idle_until_the_deadline() {
        let (master, slave) = pty_pair();
        let _ = master;
        let started = Instant::now();
        let ready = wait_any(&[slave.as_fd()], started + Duration::from_millis(30)).expect("wait");
        assert_eq!(ready, Any::Idle);
        assert!(
            started.elapsed() >= Duration::from_millis(25),
            "returned after {:?}, well before the deadline",
            started.elapsed()
        );
    }

    #[test]
    fn the_slot_that_has_data_is_named_and_not_guessed() {
        let (master0, slave0) = pty_pair();
        let (master1, slave1) = pty_pair();
        use std::io::Write as _;
        master0.write_all(b"x").expect("write slot 0");

        let ready = wait_any(
            &[slave0.as_fd(), slave1.as_fd()],
            Instant::now() + Duration::from_secs(5),
        )
        .expect("wait");
        assert_eq!(ready, Any::Ready(0), "the slot that was written is the one that is ready");
    }

    #[test]
    fn a_slot_that_went_away_is_named_and_which_one() {
        let (master0, slave0) = pty_pair();
        let (master1, slave1) = pty_pair();
        drop(master1); // the slot-1 peer, specifically, is gone

        let ready = wait_any(
            &[slave0.as_fd(), slave1.as_fd()],
            Instant::now() + Duration::from_secs(5),
        )
        .expect("wait");
        assert_eq!(ready, Any::HangUp(1), "POLLHUP on slot 1 is named as slot 1");
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p neotmux --lib tty::`
Expected: FAIL — `wait_any` and `Any` do not exist.

- [ ] **Step 3: Implement**

In `crates/neotmux/src/tty.rs`, above `wait_on`:

```rust
/// What a wait over any number of slots found. The index is the slot's position in the
/// slice it was waited over, because a hangup on one of three is not a hangup and the
/// caller is the one that knows which slot was which.
#[derive(Debug, PartialEq, Eq)]
pub enum Any {
    /// Slot `i` has something to read, or something it wants read.
    Ready(usize),
    /// Slot `i` is gone: `POLLHUP`, `POLLERR` and `POLLNVAL` are all the kernel saying the
    /// same thing about a slot that will give no more, and they are all looked at first,
    /// for the reason `wait_on` already gives.
    HangUp(usize),
    /// The deadline passed with nothing on any of them.
    Idle,
}

/// Waits over any number of slots for the first that has something, and says which one.
/// `wait_on` is its two-slot shape, the whole reason a second one exists at all; three
/// is the attached clients, the door and the panes, which is the shape that has grown.
pub fn wait_any(slot_fds: &[BorrowedFd<'_>], deadline: Instant) -> io::Result<Any> {
    loop {
        let mut watched: Vec<PollFd> = slot_fds
            .iter()
            .map(|fd| PollFd::new(*fd, PollFlags::POLLIN))
            .collect();
        match poll(
            &mut watched,
            timeout(deadline.saturating_duration_since(Instant::now())),
        ) {
            Ok(0) => return Ok(Any::Idle),
            Ok(_) => {
                let gone = PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL;
                for (slot, watched) in watched.iter().enumerate() {
                    let revents = watched.revents().unwrap_or_else(PollFlags::empty);
                    if revents.intersects(gone) {
                        return Ok(Any::HangUp(slot));
                    }
                }
                for (slot, watched) in watched.iter().enumerate() {
                    if watched
                        .revents()
                        .unwrap_or_else(PollFlags::empty)
                        .contains(PollFlags::POLLIN)
                    {
                        return Ok(Any::Ready(slot));
                    }
                }
            }
            Err(Errno::EINTR) => {
                if Instant::now() >= deadline {
                    return Ok(Any::Idle);
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p neotmux --lib tty::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/src/tty.rs
git commit -m "the wait learns any number of slots and names which one"
```


---

### Task 2: What a tock does, split from the picture it makes

**Files:**
- Modify: `crates/neotmux/src/session.rs` (the `Session` impl: `advance` at line 632, `frame` at 567; `forget_pane_output` at 608)
- Test: Modify: `crates/neotmux/tests/headless.rs` (a new test beside its existing frame-driven ones)

**Interfaces:**
- Consumes: nothing new.
- Produces: `Session::apply(&mut self, tock: &Tock) -> anyhow::Result<Apply>`, `struct Apply { steps: Vec<Step>, end: Option<End> }`; `Session::frame_with(&mut self, overlay: &[DrawCmd], notices: Vec<Notice>) -> Frame` (the existing `frame(notices)` becomes `frame_with(overlay, notices)`); `Session::wants_output(&mut self, wanted: bool)`; `Session::is_dirty(&self) -> bool`. `advance` is kept and redefined on them, because `tests/headless.rs` drives a session through it.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux/tests/headless.rs`

```rust
/// The split in one test: the part a tock does to a session, which is the part shared,
/// and the part it is shown in, which is the part the server makes per client. Two
/// tocks, one session, both applies, both frames.
#[test]
fn two_applies_are_two_stops_on_the_way_to_two_frames() {
    let (_dir, _wakeup, mut session, _plugins) = halves(&[("init.lua", "")]);

    // A tock with no ops is a clean apply: steps empty, no end — and no field on the
    // apply for the picture, which is what the split is, the picture not being the
    // apply's to know about.
    let tock = neotmux_proto::msg::Tock {
        ops: Vec::new(),
        overlay: Vec::new(),
        notices: Vec::new(),
        reserved: neotmux_proto::Reserved::default(),
        gutter: 0,
        wants_pane_output: false,
    };
    let application = session.apply(&tock).expect("apply");
    assert!(application.steps.is_empty());
    assert!(application.end.is_none());

    // `frame_with` takes overlay and notices as its own: two calls, one session, the
    // same panes and size — the picture is the caller's, not the session's, and is the
    // whole of how two clients see the one it is in.
    let first = session.frame_with(&Vec::new(), Vec::new());
    let second = session.frame_with(&Vec::new(), Vec::new());
    assert_eq!(first.size, second.size, "the same session, both frames");
    assert_eq!(first.panes.len(), second.panes.len(), "the session's panes, both times");
    for p in &first.panes {
        assert!(second.panes.iter().any(|q| q.pane == p.pane));
    }

    // `wants_output` is the whole of the server's question of it: the session holds what
    // it is told, the OR being the server's to keep, not the session's to keep.
    session.wants_output(true);
    session.wants_output(false);
    let _ = session.is_dirty();
}
```

(The test is deliberately plain about the API shape: `apply` has no `frame` field, `frame_with` owns the overlay and notices it renders, and `wants_output` and `is_dirty` are the setter and the reader the server's loop is written against. The two `frame_with` calls with empty overlays are the same picture, which is the assertion on the split, not on the overlay; the overlay is what Task 4 shows with a real one.)


- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --test headless two_applies`
Expected: FAIL — `apply`, `frame_with`, `wants_output`, `is_dirty` do not exist.

- [ ] **Step 3: Implement in `session.rs`**

In the `Session` impl:

```rust
    /// What applying a tock did to the session: the shared part, ops and geometry and
    /// the end, in the order they were asked. Not the picture — the picture is the
    /// caller's to make of this, with the overlay and notices the caller knows to be
    /// this client's own, and this is the whole of why a tock with two clients is a
    /// tock with one.
    pub fn apply(&mut self, tock: &Tock) -> anyhow::Result<Apply> {
        if let Some(end) = self.apply_ops(&tock.ops)? {
            return Ok(Apply {
                steps: std::mem::take(&mut self.steps),
                end: Some(end),
            });
        }
        let reserved_changed = self.reserved_changed(tock.reserved);
        let gutter_changed = self.gutter_changed(tock.gutter);
        if reserved_changed || gutter_changed {
            self.apply_geometry()?;
        }
        let eof = self.ended;
        Ok(Apply {
            steps: std::mem::take(&mut self.steps),
            end: eof.then_some(End::Eof),
        })
    }

    /// The session's picture with a client's overlay and notices on it. The overlay and
    /// notices are the client's, passed in because they are this client's and the last
    /// ones sent are not necessarily this client's; the panes and the rects are the
    /// session's, as everything else about the picture is.
    pub fn frame_with(&mut self, overlay: &[DrawCmd], notices: Vec<Notice>) -> Frame<'_> {
        // The shown window's panes only, for the reason `frame` already gives.
        let shown: Vec<PaneId> = self.windows.current().rects.keys().copied().collect();
        self.shells.snapshot(shown.into_iter());
        self.dirty = false;

        let layout = self.windows.current();
        Frame {
            size: self.surface.size,
            panes: layout
                .showable()
                .filter_map(|(pane, rect)| {
                    Some(PaneFrame {
                        pane,
                        rect,
                        grid: self.shells.grid(pane)?,
                    })
                })
                .collect(),
            rects: &layout.rects,
            cursor: layout.focus(),
            overlay,
            notices,
        }
    }

    /// Whether the session changed in the way a frame is for, whatever the client's own
    /// overlay is about to add on top. A client decides whether to force one; the
    /// session only answers this half.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The one subscription that is the session's and not a client's: the pipe is the
    /// session's, and a client asking for it is a client asking for the session to keep
    /// reading it. The server holds the OR of who is asking, and this is how it says so.
    pub fn wants_output(&mut self, wanted: bool) {
        self.wants_pane_output = wanted;
    }
```

Add the struct, beside `Pass`:

```rust
/// What one tock did to a session, without the picture it is shown in.
pub struct Apply {
    /// What applying the tock's ops did to the session, in order, for the reason `Pass`
    /// already gives it.
    pub steps: Vec<Step>,
    /// Why the session is over, if it is: a `Quit` a client's op raised, or an `Eof` the
    /// last of its shells just went.
    pub end: Option<End>,
}
```

Redefine `advance` on the two (keeping the overlay/notices bookkeeping it has, because `headless.rs` and the `adopt` path use it as the single-client pass):

```rust
    /// The single-client frame: a tock applied, and the picture it made, with the overlay
    /// and notices the tock carried, which are this client's because it is the only one.
    /// The server's frame with more than one is the same two calls, in the same order,
    /// the picture made once per client — which is to say, `frame` and `advance` are one
    /// and the same, read from the server's side or the client's.
    pub fn advance(&mut self, tock: &Tock) -> anyhow::Result<Pass<'_>> {
        self.wants_output(tock.wants_pane_output);
        let application = self.apply(tock)?;
        if let Some(end) = application.end {
            return Ok(Pass {
                steps: application.steps,
                frame: None,
                end: Some(end),
            });
        }
        // The dirty the overlay and notices add, the reason they are here at all.
        let overlay_changed = tock.overlay != self.last_overlay;
        let notices_changed = tock.notices != self.last_notices;
        self.last_overlay.clone_from(&tock.overlay);
        self.last_notices.clone_from(&tock.notices);
        if overlay_changed || notices_changed {
            self.dirty = true;
        }
        let frame = if self.dirty {
            Some(self.frame_with(&self.last_overlay, tock.notices.clone()))
        } else {
            None
        };
        Ok(Pass {
            steps: application.steps,
            frame,
            end: None,
        })
    }
```

Keep `forget_pane_output` as `self.wants_output(false)`.

Note for the executor: the original `advance` checked `eof` *after* the overlay/notices comparison and forced a frame when it was set (`if self.dirty || eof`). The redefinition above folds the `Eof` path into `apply`'s `end`, which drops that final frame. Restore it here exactly: when `application.end == Some(End::Eof)`, force the frame through the same `dirty` the original had, i.e.

```rust
        if application.end == Some(End::Eof) {
            let frame = Some(self.frame_with(&self.last_overlay, tock.notices.clone()));
            return Ok(Pass { steps: application.steps, frame, end: Some(End::Eof) });
        }
```

before the overlay bookkeeping. (The original did `end: eof.then_some(End::Eof)` *and* `frame: Some(self.frame(...))` when `eof`; the redefinition must do both, not one.)

- [ ] **Step 4: Run the headless suite and the session unit tests**

Run: `cargo test -p neotmux --test headless && cargo test -p neotmux --lib session::`
Expected: PASS. `advance` is the single-client frame, so everything that drove a session through it is driving it through the same two calls again.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/src/session.rs crates/neotmux/tests/headless.rs
git commit -m "what a tock does to a session, split from the picture it makes"
```

---

### Task 3: The frame with everybody

**Files:**
- Modify: `crates/neotmux/src/server.rs` (whole file — it is 381 lines of the one-client loop and is being widened, not patched)
- Test: Modify: `crates/neotmux/tests/socket.rs:307` (`a_second_client_is_refused_and_says_why` is flipped, because what it refused is now accepted)

**Interfaces:**
- Consumes: `tty::wait_any`, `tty::Any` (Task 1); `Session::apply`/`frame_with`/`wants_output`/`is_dirty` (Task 2); `sock`, `link`, `reads`, `wakeup`, `wire` as they already are.
- Produces: `serve(listener, shutdown) -> anyhow::Result<()>` with the same signature — so nothing outside `server.rs` changes shape — and the `Attached` state the frame loop keeps per client, visible only inside this file.

- [ ] **Step 1: Flip the refused test**

In `crates/neotmux/tests/socket.rs`, replace `a_second_client_is_refused_and_says_why` (line 307) with:

```rust
/// Accepted and attached, rather than refused: the whole of what the second client used
/// to be turned away for is now being let in, and what it gets for that is the session.
#[test]
fn a_second_client_is_attached_side_by_side_with_the_first() {
    let runtime = TestRuntime::new();
    let first = started(runtime.path());

    // The second connection is greeted, adopts the session, and is shown a frame of it
    // — the first, because no client has been looking since. What it was told when it
    // was refused is exactly what it is not told now.
    let mut second = greeted(socket_in(runtime.path()));
    send_message(&mut second, &wire::encode_to_server(&ToServer::Attach(a_reattach())));
    let got = read_one_message(&mut second).expect("the second client is shown something");
    let decoded: neotmux_proto::OwnedToClient =
        neotmux_proto::wire::decode_to_client(&got).expect("the frame decodes");
    assert!(
        matches!(decoded, neotmux_proto::OwnedToClient::Tick(_)),
        "the second client is shown the session, not refused it: {decoded:?}"
    );
    drop(first);
}
```

(Match the test file's existing import style — `wire` and the message types — and build `a_reattach()` the way `a_reattached_client_builds_the_same_float_again` builds its attach on line 342, with `Attach { ..a_reattach() }` if it uses that shorthand. `read_one_message` is the file's existing reader; if it is named differently there, use that name. If the first client must still be *in the frame* when the second attaches — which is the test's point — hold `first`'s stream open and idle until the assertion, the way the refused test already held it.)

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --test socket a_second_client`
Expected: FAIL — the second client is still refused.

- [ ] **Step 3: Rewrite `server.rs`**

The whole file, one listing. Everything after this step that the old file had and this
one does not — `accept`, `attached`, `converse`, `apply_pending`, `await_tock`, `send`,
`turn_away`, the `BUSY` constant — is deleted in Step 4, and `taken` is kept, because it
is the door.

```rust
//! The process that holds the session.
//!
//! It has no terminal: it owns the panes, their children, the tree they are laid out in
//! and the terminal state behind each of them, and it never looks at it. Every frame is
//! a conversation with the clients — what happened, what the plugins asked for, what to
//! show — and a pane read one of them makes is answered in the one window the server is
//! already waiting for that client to speak, on that client's link and no other, so a
//! read is never a question with no one to answer it and is never picked out of a stream
//! of frames.
//!
//! Its life is the session's: when a client goes away the session is kept until its last
//! shell exits, still reading its children, so a disconnect is not a crash. A client that
//! arrives afterwards is shown that same session — and so is one that arrives while any
//! one else is looking, the one the old code refused with a reason, and the two
//! difference being the one the other is not in. Which clients get to look at a session
//! is as far as this file goes; that a session sits waiting at all is the rest of it.

use std::os::fd::AsFd as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use neotmux_proto::msg::{Attach, Done, End, Tock, ToClient, ToServer};
use neotmux_proto::wire;
use neotmux_proto::{DrawCmd, Level, Notice};

use neotmux_term::wakeup::{Waker, Wakeup, wakeup};

use crate::link::Link;
use crate::logging;
use crate::reads;
use crate::session::Apply;
use crate::session::Session;
use crate::signals::ShutdownSignal;

/// The frame's cadence, and so the closest together two frames can be. The same number
/// the one-process loop used, for the same reason: it is a property of the loop rather
/// than of whatever the wait happens to do.
const FRAME: Duration = Duration::from_millis(16);

/// How long a server with no session waits for the client that started it. It is the
/// client's own patience: past that, nobody is coming and there is nothing to serve.
const STARTUP: Duration = Duration::from_secs(10);

/// One slice of the tock wait. The tock wait has no deadline of its own — a client takes
/// as long as it takes, and the one-client loop had the same property, its `recv`
/// blocking until the client sent or stopped — so a client that stops sending is a
/// client whose connection stops, and the slice is only the length of one `poll` the
/// way it is, because the wait has to be askable of the door and the panes on the way
/// rather than a `recv` a client holds forever.
const TOCK: Duration = Duration::from_secs(60);

/// One client the frame reaches, and everything the server has to remember about it.
/// The picture part is per client and not the session's, which is to say the overlay
/// and notices a frame carries are this client's own, and the question "has it changed
/// since the frame it was last shown with" is the same question with its answer on the
/// client.
struct Attached {
    link: Link,
    /// Pane output this client asked for on its last tock. The session reads the pipe if
    /// any attached client is asking, and stops when the last of them goes — the OR, not
    /// the last tock, which is the difference a second client makes.
    wants: bool,
    /// The overlay and notices its last frame carried.
    last_overlay: Vec<DrawCmd>,
    last_notices: Vec<Notice>,
    /// No frame has reached this connection yet, so its first is forced: it was not told
    /// the frames since the last client's, and "nothing changed" is a lie to someone who
    /// has seen nothing.
    first: bool,
    /// The client has gone, in this frame. Recorded rather than dropped, because the
    /// frame's slot is still a slot the wait is waiting over, and dropping it out from
    /// under a wait is what this flag prevents.
    gone: bool,
    /// The tock this client gave differs, in either of the two things a frame carries
    /// that are this client's own, from the frame it was last shown with.
    overlay_dirty: bool,
    notices_dirty: bool,
}

pub fn serve(listener: &UnixListener, shutdown: &ShutdownSignal) -> anyhow::Result<()> {
    // Polled rather than blocked on, so the frame loop can answer a client instead of
    // leaving it in the backlog wondering.
    listener
        .set_nonblocking(true)
        .context("make the socket pollable")?;
    let (wakeup, waker) = wakeup().context("open the panes' wakeup")?;
    let mut session: Option<Session> = None;
    let mut clients: Vec<Attached> = Vec::new();
    let born = Instant::now();

    loop {
        if shutdown.raised().is_some() {
            return Ok(());
        }
        if clients.is_empty() {
            match await_client(
                &mut clients,
                &mut session,
                listener,
                &wakeup,
                &waker,
                shutdown,
                born,
            )? {
                true => continue,
                // Over: signalled, out of its patience with no session, or the last
                // shell of a session no client is holding went.
                false => return Ok(()),
            }
        }
        if frame(&mut session, &mut clients, listener, &wakeup, &waker, shutdown)? {
            return Ok(());
        }
    }
}

/// A session with no client is kept for as long as its children go, and a server that
/// never got a client is not a session worth keeping. Both of those are decided here,
/// and the one that decided them is the one that says the server is over.
///
/// `true` is a client that came in and the session going on; `false` is the server being
/// over, and the loop does not run again.
fn await_client(
    clients: &mut Vec<Attached>,
    session: &mut Option<Session>,
    listener: &UnixListener,
    wakeup: &Wakeup,
    waker: &Waker,
    shutdown: &ShutdownSignal,
    born: Instant,
) -> anyhow::Result<bool> {
    // A session with a client that has left waits for ever: that is what detach means.
    // A server that never had one is not a session worth keeping, so `STARTUP` still
    // applies.
    let give_up = session.is_none().then_some(born + STARTUP);
    loop {
        if shutdown.raised().is_some() {
            return Ok(false);
        }
        let frame_end = Instant::now() + FRAME;
        let ready = crate::tty::wait_on(
            std::os::fd::AsFd::as_fd(listener),
            session.is_some().then(|| wakeup.as_fd()),
            give_up.map_or(frame_end, |at| at.min(frame_end)),
        )
        .context("wait for a client")?;
        if ready == crate::tty::Ready::Idle
            && give_up.is_some_and(|at| Instant::now() >= at)
        {
            return Ok(false);
        }
        if let Some(session) = session.as_deref_mut() {
            // Only once the poll has said so: the nudge is a pipe, and reading an empty
            // one is a wait with nothing to end it.
            if ready == crate::tty::Ready::Output {
                wakeup.clear();
            }
            if session.idle()? {
                return Ok(false);
            }
        }
        if ready == crate::tty::Ready::Input
            && let Some(stream) = taken(listener)?
        {
            if let Some(client) = attach_new(stream, session, waker)? {
                if let Some(session) = session {
                    // The OR the other clients already set — which here is none, so the
                    // new one alone — or, in the door case of `frame`, the rest of them.
                    session.wants_output(clients.iter().any(|c| c.wants) || client.wants);
                }
                clients.push(client);
                return Ok(true);
            }
            // The connection did not come to attach, and what a client does to its own
            // connection may not end anybody's session, the rule the old `attached`
            // kept: it is dropped, and the server goes back to waiting.
        }
    }
}

/// A client waiting on the socket, if there is one. The listener is non-blocking, so this
/// can be asked at any point of a frame without risking one.
fn taken(listener: &UnixListener) -> anyhow::Result<Option<UnixStream>> {
    match listener.accept() {
        Ok((stream, _)) => Ok(Some(stream)),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(err).context("accept a client"),
    }
}

/// The new client, greeted and adopted: the one path that still says no to a connection
/// whose first message is not an attach, for the reason the old `attached` gave, and one
/// that says yes — rather than no, with a reason the client would not have believed —
/// to a client a session already has. The caller, who has the other clients, sets the
/// `wants_pane_output` OR, because `attach_new` does not hold them.
fn attach_new(
    stream: UnixStream,
    session: &mut Option<Session>,
    waker: &Waker,
) -> anyhow::Result<Option<Attached>> {
    let link = match Link::open(stream) {
        Ok(link) => link,
        // A client built against another protocol, or one that hung up mid-handshake.
        // Its own end says so; this one goes back to waiting.
        Err(err) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("a client could not be greeted: {err}"),
            );
            return Ok(None);
        }
    };
    let attach = match first_message(&mut link)? {
        Some(attach) => attach,
        None => return Ok(None),
    };
    let wants = attach.wants_pane_output;
    match session.as_mut() {
        Some(mut session) => session.adopt(&attach)?,
        None => {
            let session = Session::new(&attach, waker.clone())?;
            *session = Some(session);
        }
    }
    Ok(Some(Attached {
        link,
        wants,
        last_overlay: Vec::new(),
        last_notices: Vec::new(),
        first: true,
        gone: false,
        overlay_dirty: false,
        notices_dirty: false,
    }))
}

/// The client's first message, and the reason it was not an attach, if it was not. The
/// one place in the code a first message is read, which is where the rules the old
/// `attached` had live, because they are the same rules.
fn first_message(link: &mut Link) -> anyhow::Result<Option<Attach>> {
    let message = match link.recv() {
        Ok(Some(message)) => message,
        Ok(None) => return Ok(None),
        Err(err) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("a client's first message could not be read: {err}"),
            );
            return Ok(None);
        }
    };
    match wire::decode_to_server(&message) {
        Ok(ToServer::Attach(attach)) => Ok(Some(attach)),
        Ok(other) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("a client's first message was {other:?} rather than an attach"),
            );
            Ok(None)
        }
        Err(err) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("a client's first message could not be decoded: {err}"),
            );
            Ok(None)
        }
    }
}

/// The frame, with every client the session has at the moment the frame starts: one
/// tick, each client's tock, and what all of it did, in the order the clients attached.
///
/// `true` is the server being over — the session ending on a `Quit` or its last shell —
/// and `false` is the frame being over, one client fewer or the same, the session going
/// on.
fn frame(
    session: &mut Option<Session>,
    clients: &mut Vec<Attached>,
    listener: &UnixListener,
    wakeup: &Wakeup,
    waker: &Waker,
    shutdown: &ShutdownSignal,
) -> anyhow::Result<bool> {
    // The slots the frame waits over, in order: a client per attachment, in the order
    // the clients attached, the door, and the panes. The tock is read off the slot of
    // the client it is, and the door and the panes owe one to nobody.
    enum Slot {
        Client(usize),
        Door,
        Panes,
    }
    let mut slots: Vec<Slot> = (0..clients.len()).map(|i| Slot::Client(i)).collect();
    slots.push(Slot::Door);
    slots.push(Slot::Panes);
    let mut slot_fds: Vec<std::os::fd::BorrowedFd> =
        clients.iter().map(|client| client.link.as_fd()).collect();
    slot_fds.push(listener.as_fd());
    slot_fds.push(wakeup.as_fd());

    // The tick, once, shown to every client the frame is for. A client that cannot read
    // it is a client that has gone, the rule the server has kept since M3, per
    // connection.
    let Some(session) = session.as_deref_mut() else {
        clients.retain(|client| !client.gone);
        return Ok(false);
    };
    let tick = session.tick()?;
    let mut expected: Vec<usize> = Vec::new();
    {
        let session = session; // the borrow below is the whole of what the tick holds
        for (i, client) in clients.iter().enumerate() {
            if client
                .link
                .send(&wire::encode_to_client(&ToClient::Tick(&tick)))
                .is_ok()
            {
                expected.push(i);
            } else {
                client.gone = true;
            }
        }
        drop(session);
    }
    drop(tick);
    if expected.is_empty() {
        clients.retain(|client| !client.gone);
        return Ok(false);
    }

    // The tocks, and whatever each client says on the way to its tock. Both of the rules
    // the old loop kept — a read answered on arrival, on the link it came on and nowhere
    // else; a key no client's plugins consumed is the session's, whichever client it
    // came from — with only the second having more than one client to come from.
    let mut tocks: Vec<Option<Tock>> = vec![None; clients.len()];
    'collect: loop {
        if shutdown.raised().is_some() {
            return Ok(true);
        }
        if !expected
            .iter()
            .any(|&i| !clients[i].gone && tocks[i].is_none())
        {
            break 'collect;
        }
        match crate::tty::wait_any(&slot_fds, Instant::now() + TOCK)? {
            crate::tty::Any::Ready(i) => match slots[i] {
                Slot::Client(i) => {
                    if !clients[i].link.fill()? {
                        clients[i].gone = true;
                        continue 'collect;
                    }
                    let Some(message) = clients[i].link.take()? else { continue 'collect };
                    match wire::decode_to_server(&message) {
                        Ok(ToServer::Tock(tock)) if tocks[i].is_none() => {
                            // The client's own question, before the tock is handed over:
                            // the frame it is for has to know whether it is one to show
                            // it with.
                            clients[i].overlay_dirty =
                                tock.overlay != clients[i].last_overlay;
                            clients[i].notices_dirty = tock.notices != clients[i].last_notices;
                            clients[i].wants = tock.wants_pane_output;
                            tocks[i] = Some(tock);
                        }
                        Ok(ToServer::Key(key)) => {
                            let Some(session) = session.as_deref_mut() else { break 'collect };
                            if session.key(key).is_err() {
                                clients[i].gone = true;
                            }
                        }
                        Ok(ToServer::Resize(size)) => {
                            let Some(session) = session.as_deref_mut() else { break 'collect };
                            if session.resize(size).is_err() {
                                clients[i].gone = true;
                            }
                        }
                        Ok(ToServer::Read(read)) => {
                            let Some(session) = session.as_deref_mut() else { break 'collect };
                            let reading = reads::answer(session.terminals().as_ref(), read);
                            if clients[i]
                                .link
                                .send(&wire::encode_to_client(&ToClient::Reading(&reading)))
                                .is_err()
                            {
                                clients[i].gone = true;
                            }
                        }
                        Ok(ToServer::Tock(_)) => {
                            // A tock the frame already collected for this client, or a
                            // tock for a frame this loop is not inside of: ignored, the
                            // rule the old `await_tock` kept.
                        }
                        Ok(ToServer::Attach(_)) => {
                            // A second attach is not something the session can act on out
                            // here, the rule the old `apply_pending` kept.
                        }
                        Err(err) => {
                            // Undecodable on one client's link is that client's doing and
                            // costs its connection and nothing else.
                            logging::log(
                                Level::Warn,
                                "neotmux-server",
                                &format!(
                                    "an attached client sent something that would not decode: {err}"
                                ),
                            );
                            clients[i].gone = true;
                        }
                    }
                }
                Slot::Door => {
                    let Some(stream) = taken(listener)? else { continue 'collect };
                    // `frame` holds the session as an `Option` for this arm, and the
                    // caller that has the rest of the clients is this one, so the OR is
                    // the same one the door case of `await_client` sets.
                    let Some(client) = attach_new(stream, &mut session, waker)? else {
                        continue 'collect
                    };
                    if let Some(session) = session.as_deref_mut() {
                        session.wants_output(
                            clients.iter().any(|c| c.wants) || client.wants,
                        );
                    }
                    clients.push(client);
                    // The one adopted here is not in `expected`: it gets its first tick
                    // at the next frame, and a tock it sends this frame is a tock this
                    // loop does not collect, which is to say, the same rule as the door
                    // cases of `apply_pending` and `await_tock` ignoring one.
                }
                Slot::Panes => {
                    // A shell that wrote is not a tock: the panes are drained to keep the
                    // pipe from filling, and the frame goes on, because the tocks are the
                    // frame. The old loop broke on this in its post-done idle, not in
                    // its tock wait, which is where this one is.
                    wakeup.clear();
                }
            },
            crate::tty::Any::HangUp(i) => match slots[i] {
                Slot::Client(i) => clients[i].gone = true,
                _ => {}
            },
            crate::tty::Any::Idle => {}
        }
    }

    // What all of it did, in the order the clients attached. A client that is gone
    // before this had its tock is a client whose tock is not applied, and the session
    // goes on without it.
    let mut steps: Vec<neotmux_proto::msg::Step> = Vec::new();
    let mut ended: Option<End> = None;
    let mut detacher: Option<usize> = None;
    for &i in &expected {
        if clients[i].gone {
            continue;
        }
        let Some(tock) = tocks[i].as_ref() else { continue };
        let Some(session) = session.as_deref_mut() else { continue };
        let application: Apply = session.apply(tock)?;
        steps.extend(application.steps);
        if let Some(end) = application.end {
            match end {
                End::Detach => detacher = Some(i),
                other => ended = Some(other),
            }
        }
    }

    // The picture, per client, because the picture is per client. The session's dirt is
    // the same for all of them and is read before the first render, because a render is
    // what clears it; the overlay and the notices are each one's own.
    let needs_session = session.as_deref_mut().is_some();
    let _ = needs_session;
    if let Some(session) = session.as_deref_mut() {
        let session_dirty = session.is_dirty();
        let force = matches!(ended, Some(End::Eof));
        for &i in &expected {
            let client = &mut clients[i];
            if client.gone {
                continue;
            }
            let Some(tock) = tocks[i].as_ref() else { continue };
            let needs = force
                || session_dirty
                || client.first
                || client.overlay_dirty
                || client.notices_dirty;
            let frame_ref = if needs {
                Some(session.frame_with(&tock.overlay, tock.notices.clone()))
            } else {
                None
            };
            let end = if detacher == Some(i) {
                Some(End::Detach)
            } else {
                ended
            };
            let done = Done {
                steps: steps.clone(),
                frame: frame_ref,
                end,
            };
            if client.link.send(&wire::encode_to_client(&ToClient::Done(&done))).is_err() {
                client.gone = true;
            }
            client.last_overlay = tock.overlay.clone();
            client.last_notices = tock.notices.clone();
            client.overlay_dirty = false;
            client.notices_dirty = false;
            client.first = false;
            if detacher == Some(i) {
                // The one that is detaching is out, and its connection is about to close
                // for the reason it asked to leave: drop it here rather than at the next
                // frame's first send.
                client.gone = true;
            }
            drop(frame_ref);
        }
    }

    // The gone are out, and the OR is taken over the rest, the one place it is taken
    // and not one of the two it is not.
    clients.retain(|client| !client.gone);
    if let Some(session) = session.as_deref_mut() {
        session.wants_output(clients.iter().any(|client| client.wants));
    }

    if ended.is_some() {
        return Ok(true);
    }

    // The rest of the frame's time belongs to whatever the clients have to say and to
    // the panes: a key reaches its shell the moment it arrives, and the panes' output
    // is the cue to start the next frame rather than to wait the deadline out.
    let frame_end = Instant::now() + FRAME;
    let mut slots: Vec<Slot> = (0..clients.len()).map(|i| Slot::Client(i)).collect();
    slots.push(Slot::Door);
    slots.push(Slot::Panes);
    let mut slot_fds: Vec<std::os::fd::BorrowedFd> =
        clients.iter().map(|client| client.link.as_fd()).collect();
    slot_fds.push(listener.as_fd());
    slot_fds.push(wakeup.as_fd());
    loop {
        if shutdown.raised().is_some() {
            return Ok(true);
        }
        if Instant::now() >= frame_end {
            break;
        }
        match crate::tty::wait_any(&slot_fds, frame_end)? {
            crate::tty::Any::Ready(i) => match slots[i] {
                Slot::Client(i) => {
                    if !clients[i].link.fill()? {
                        clients[i].gone = true;
                        continue;
                    }
                    let Some(message) = clients[i].link.take()? else { continue };
                    match wire::decode_to_server(&message) {
                        Ok(ToServer::Key(key)) => {
                            if let Some(session) = session.as_deref_mut() {
                                session.key(key)?;
                            }
                        }
                        Ok(ToServer::Resize(size)) => {
                            if let Some(session) = session.as_deref_mut() {
                                session.resize(size)?;
                            }
                        }
                        Ok(ToServer::Read(read)) => {
                            if let Some(session) = session.as_deref_mut() {
                                let reading =
                                    reads::answer(session.terminals().as_ref(), read);
                                if clients[i]
                                    .link
                                    .send(&wire::encode_to_client(&ToClient::Reading(
                                        &reading,
                                    )))
                                    .is_err()
                                {
                                    clients[i].gone = true;
                                }
                            }
                        }
                        Ok(ToServer::Tock(_)) | Ok(ToServer::Attach(_)) => {
                            // A reply to a tick this loop is not inside of, or a second
                            // attach: neither is something the session can act on out
                            // here, the rule the old `apply_pending` kept.
                        }
                        Err(err) => {
                            logging::log(
                                Level::Warn,
                                "neotmux-server",
                                &format!(
                                    "an attached client sent something that would not decode: {err}"
                                ),
                            );
                            clients[i].gone = true;
                        }
                    }
                }
                Slot::Door => {
                    let Some(stream) = taken(listener)? else { continue };
                    let Some(client) = attach_new(stream, &mut session, waker)? else {
                        continue
                    };
                    if let Some(session) = session {
                        session.wants_output(
                            clients.iter().any(|c| c.wants) || client.wants,
                        );
                    }
                    clients.push(client);
                }
                Slot::Panes => {
                    wakeup.clear();
                    break;
                }
            },
            crate::tty::Any::HangUp(i) => match slots[i] {
                Slot::Client(i) => clients[i].gone = true,
                _ => {}
            },
            crate::tty::Any::Idle => {}
        }
    }
    clients.retain(|client| !client.gone);
    if let Some(session) = session.as_deref_mut() {
        session.wants_output(clients.iter().any(|client| client.wants));
    }
    Ok(false)
}
```

Two lines in that listing are load-bearing and the executor should read them before compiling:

1. `Slot::Door` in both wait loops calls `attach_new(stream, &mut session, waker)` with
   `session` the function parameter — an `&mut Option<Session>` — because `attach_new`
   wants exactly that, and a client adopted mid-frame takes the `Option` path the
   frame's first line does not.
2. The picture loop reads `session.is_dirty()` once, before any `frame_with`, because a
   render clears the dirt it is the answer to, and two clients on the same render would
   read it twice.

- [ ] **Step 4: Delete the one-client paths and build**

Delete from `server.rs`: `accept`, `attached`, `converse`, `apply_pending`,
`await_tock`, `send`, `turn_away`, and the `BUSY` constant — the new loop is the whole
file now, and each of those is one of the paths it supersedes, the way the new listing
says. `session.rs` keeps `forget_pane_output` one more step: nothing in the crate calls
it any more, and the step that is allowed to make it dead code is this one, so delete
it from `session.rs` too (`wants_output(false)` is what was left of it, the one
`frame`-internal place the OR is set to the empty set).

Run: `cargo build --workspace && cargo clippy -p neotmux --all-targets -- -D warnings`
Expected: PASS. An unused import or an unused method the deletion was supposed to take
with it is the error to expect, and the name it names is the one to delete next.


- [ ] **Step 5: The whole server-facing suite**

Run: `cargo test -p neotmux --test socket && cargo test -p neotmux --test headless && cargo test -p neotmux --lib && cargo test -p neotmux --test sessions`
Expected: PASS. The flipped `a_second_client_is_attached_side_by_side_with_the_first` is the one that fails while the server is still one-client, and passes from this point on. `a_detached_session_survives_a_connection_that_never_attaches` and `a_detached_session_survives_a_connection_whose_first_message_is_not_an_attach` pass unchanged: a connection that does not attach still costs nothing, which is the rule that did not change.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux/src/server.rs crates/neotmux/tests/socket.rs
git commit -m "the frame with everybody: one tick, each one's tock, and the done both of them are"
```

---

### Task 4: Two clients, in one runtime, and the shapes a person sees

**Files:**
- Modify: `crates/neotmux/tests/socket.rs` — every test of this task is in this file, because this is where the two-client verbs already live: `TestRuntime` (an isolated `$XDG_RUNTIME_DIR` that outlives the test and ends whatever session a test left behind on the way out), `Client` (`start(runtime, shell)`, `send`, `wait_for_output`, `kill`, `wait_for_exit`), `socket_in`, `greeted` at line 417, `send_message` at line 426, `an_attach()` at line 446, and `started()` at line 239, the one that proves the round trip.

**Interfaces:**
- Consumes: Tasks 1–3; the `Client` wrapper above; the raw-link shape the file already keeps at lines 614–635 — a message is four length bytes and then its body, and `wire::decode_to_client` reads it, the way `a_client_that_greets_and_leaves_without_attaching_costs_nothing` already answers one.
- Produces: the five tests below, whose names the spec's §10 list says.

- [ ] **Step 1: The raw-link verbs the protocol tests stand on**

In `tests/socket.rs`, beside the other raw-link helpers (`greeted`, `send_message`, `an_attach`):

```rust
/// One message off a raw link, the way the file's other raw test reads one: four length
/// bytes, then the body, then `wire` says what it is.
fn read_message(stream: &mut UnixStream) -> neotmux_proto::OwnedToClient {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).expect("the server owes a length");
    let body = (&mut *stream).take(u32::from_le_bytes(len) as u64).read().expect("body");
    neotmux_proto::wire::decode_to_client(&body)
        .expect("the server's message decodes")
}

/// A raw client into the session: the greet, the attach with the one it is, and the first
/// frame it is owed — so the frame it owes goes nowhere held by it.
fn raw_client(attach: neotmux_proto::msg::Attach, runtime: &TestRuntime) -> UnixStream {
    let reserved = attach.reserved;
    let gutter = attach.gutter;
    let wants = attach.wants_pane_output;
    let mut stream = greeted(socket_in(runtime.path()));
    send_message(
        &mut stream,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::msg::ToServer::Attach(attach)),
    );
    let _tick = match read_message(&mut stream) {
        neotmux_proto::OwnedToClient::Tick(t) => t,
        other => panic!("the attach was answered with {other:?}, not a tick"),
    };
    // The tock it owes, with its own word in it, and the done that owes it back.
    let tock = neotmux_proto::msg::Tock {
        ops: Vec::new(),
        overlay: Vec::new(),
        notices: Vec::new(),
        reserved,
        gutter,
        wants_pane_output: wants,
    };
    send_message(
        &mut stream,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::msg::ToServer::Tock(tock)),
    );
    let _done = read_message(&mut stream);
    stream
}

/// One whole frame for a raw client, in the order it is owed: the tick, its tock, the
/// done. A frame is a round the client is in, and this is the round, one after the
/// other.
fn frame_once(stream: &mut UnixStream) -> neotmux_proto::msg::Tick {
    let tick = match read_message(stream) {
        neotmux_proto::OwnedToClient::Tick(tick) => tick,
        other => panic!("the frame began with {other:?}, not a tick"),
    };
    let tock = neotmux_proto::msg::Tock {
        ops: Vec::new(),
        overlay: Vec::new(),
        notices: Vec::new(),
        reserved: neotmux_proto::Reserved::default(),
        gutter: 0,
        wants_pane_output: false,
    };
    send_message(
        stream,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::ToServer::Tock(tock)),
    );
    let _done = read_message(stream);
    tick
}

/// A key to the focused pane, the way a client that is not in a mode gives one: the one
/// its plugins did not consume, and so the session's.
fn type_key(stream: &mut UnixStream, ch: char) {
    send_message(
        stream,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::ToServer::Key(
            neotmux_proto::Key::plain(neotmux_proto::KeyCode::Char(ch)),
        )),
    );
}
```

(`Tick` is the name `neotmux_proto::wire::decode_to_client` has for a tick, the one `OwnedToClient::Tick(Tick)` holds, which is to say the one `neotmux-proto/src/msg.rs` at the line `a_second_client`'s test reads, and the one the two tests of this step share. `type_key`'s `Key` and `KeyCode` are the ones `crates/neotmux/tests/headless.rs` builds its `typed()` helper with at the top of that file.)

- [ ] **Step 2: The input test — the one the feature is for**

```rust
/// Two terminals in the session, and a word typed on one that the other has typed
/// nothing: the one the first's plugins did not consume, the one the second sees — the
/// whole of the feature in one sentence. The shell is `cat`, so the word is the pane's
/// own, and the pane is the session's, and the other client is looking at that pane.
#[test]
fn a_key_typed_on_one_client_is_seen_on_the_other() {
    let runtime = TestRuntime::new();
    let mut a = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    a.send("the round trip is\r");
    a.wait_for_output("the round trip is");

    let mut b = Client::start(runtime.path(), "/bin/cat");
    b.wait_for_output("the round trip is");

    a.send("typed-on-a\r");
    b.wait_for_output("typed-on-a");
    // In the other direction, because the Review Focus says whichever client: the key
    // one of them types is the session's, and the session is both of theirs.
    b.send("typed-on-b\r");
    a.wait_for_output("typed-on-b");
}
```

- [ ] **Step 3: The one-goes test**

```rust
/// One of two goes, and the other is left with the session, because the session is the
/// server's and not either client's. The one that went is gone, and the one that stayed
/// is not told the difference between "the other went" and "nothing happened" — its
/// screen still holds the one that went left behind, and the one it types goes to the
/// same pane.
#[test]
fn a_client_going_costs_the_other_nothing() {
    let runtime = TestRuntime::new();
    let mut a = Client::start(runtime.path(), "/bin/cat");
    let mut b = Client::start(runtime.path(), "/bin/cat");

    a.send("before-a-goes\r");
    b.wait_for_output("before-a-goes");
    a.kill();
    drop(a);

    // The one that is left is looking at the same pane the one that went last typed
    // into, and the one it types is the one that is, the same way the
    // `a_second_client_is_shown_the_session_the_first_one_left` test already proves,
    // for a client that leaves on purpose.
    b.wait_for_output("before-a-goes");
    b.send("still-here\r");
    b.wait_for_output("still-here");

    // The session itself is over, and the one that did not go is out of it with it: the
    // shape `the_server_outlives_a_client_that_disconnects` already answers — a connect
    // that is answered is the server itself saying so.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut answered = false;
    while Instant::now() < deadline && !answered {
        if let Ok(mut stream) = UnixStream::connect(socket_in(runtime.path())) {
            let mut hello = [0u8; neotmux_proto::wire::HELLO_LEN];
            if stream
                .write_all(&neotmux_proto::wire::encode_hello())
                .is_ok()
                && stream.read_exact(&mut hello).is_ok()
            {
                answered = true;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(answered, "the session is over, and its socket still answers");
}
```

- [ ] **Step 4: The read-routing test**

```rust
/// A question one client asks about a pane is answered on that client's link, and on no
/// other: the `Read` and the `Reading` are the two ends of the same question, and the
/// other client had not asked it. The one that is not answered is not refused an error;
/// it is simply not told, which is to say it is never, for the reason it was never
/// asked.
#[test]
fn a_read_is_answered_on_the_link_that_asked_it_only() {
    let runtime = TestRuntime::new();
    let _session = started(runtime.path());

    let mut a = raw_client(
        neotmux_proto::msg::Attach { wants_pane_output: false, ..an_attach() },
        &runtime,
    )
    .expect("a is in the session");
    let mut b = raw_client(
        neotmux_proto::msg::Attach { wants_pane_output: false, ..an_attach() },
        &runtime,
    )
    .expect("b is in the session");

    // The frame the two of them are owed, and the one they are in: one for the two.
    let _ = frame_once(&mut a);
    let _ = frame_once(&mut b);

    // A's question, on A's link: a `Read` is the one a client with plugins that are not
    // in a mode asks, and the answer is owed to the link it came on, the
    // `await_tock`-shape rule the split kept since it was two.
    send_message(
        &mut a,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::ToServer::Read(
            neotmux_proto::msg::Read::Revision {
                pane: neotmux_proto::PaneId::from(1),
            },
        )),
    );
    let got_a = read_message(&mut a);
    assert!(
        matches!(
            got_a,
            neotmux_proto::OwnedToClient::Reading(neotmux_proto::msg::Reading::Revision(
                Some(_)
            ))
        ),
        "A's question is answered on A's link: {got_a:?}"
    );

    // B did not ask, and B is not told: its patience is the one the file's
    // `a_client_that_greets_and_leaves_without_attaching_costs_nothing` uses, and it is
    // short, because a link that has not been answered is not a link that is slow.
    b.set_read_timeout(Some(Duration::from_millis(500)))
        .expect("b takes a patience");
    let mut len = [0u8; 4];
    let b_said = loop {
        match b.read_exact(&mut len) {
            Ok(_) => break true,
            Err(err)
                if err
                    .kind()
                    .eq(&std::io::ErrorKind::WouldBlock)
                    || err
                        .kind()
                        .eq(&std::io::ErrorKind::TimedOut) =>
            {
                break false
            }
            Err(_) => break true,
        }
    };
    assert!(
        !b_said,
        "the link that did not ask was answered, or at least was told it was"
    );
}
```

(B's read is timed out before being read, because a link that answers at all is a link that has been, and a timeout on a link that has not been told is the whole of what "B was not told" means. The two clients' `frame_once` calls before A's question are the two the shape needs: a frame is owed to every client that is in it, and the one that is waiting for it is the one it is owed to.)

- [ ] **Step 5: The wants test**

```rust
/// `wants_pane_output` is the OR of the attached clients, and not the last one to say
/// so: the client that asks is the one that keeps the pane delivered while it is in it,
/// and the one that does not ask does not stop the one that does, and the delivery
/// stops with the last one that asked, and not with one before it.
#[test]
fn wants_is_the_or_of_the_clients_and_not_the_last_to_say_so() {
    let runtime = TestRuntime::new();
    let _session = started(runtime.path());

    let mut a = raw_client(
        neotmux_proto::msg::Attach {
            wants_pane_output: true,
            ..an_attach()
        },
        &runtime,
    )
    .expect("a, the one that is asking");
    let mut b = raw_client(
        neotmux_proto::msg::Attach {
            wants_pane_output: false,
            ..an_attach()
        },
        &runtime,
    )
    .expect("b, the one that is not");
    let _ = frame_once(&mut a);
    let _ = frame_once(&mut b);

    // A says a word, and the one it says it on is A: the pane's event is on A's next
    // `Tick`, because A's `wants` is in the OR while A is in it — and B's `false` did
    // not win, which is the one a last-`Tock`-wins shape can say and the OR cannot.
    type_key(&mut a, "w");
    let tick_a = frame_once(&mut a);
    let tick_b = frame_once(&mut b);
    assert!(
        tick_a
            .steps
            .iter()
            .any(|step| matches!(
                step,
                neotmux_proto::msg::Step::Event(neotmux_proto::Event::PaneOutput { .. })
            )),
        "the pane A's shell wrote is in the steps of A's next tick, while A is asking: {tick_a:?}"
    );
    let _ = tick_b;

    // A goes, and the last one asking goes with it: the OR is the empty set now, and A's
    // `true` is not the last word in it, and B's `false` is not the first.
    drop(a);
    let _ = frame_once(&mut b);
    type_key(&mut b, "w");
    let tick_b_after = frame_once(&mut b);
    assert!(
        !tick_b_after
            .steps
            .iter()
            .any(|step| matches!(
                step,
                neotmux_proto::msg::Step::Event(neotmux_proto::Event::PaneOutput { .. })
            )),
        "the last client that asked went with A, and the delivery went with it: {tick_b_after:?}"
    );
}
```

(`Step::Event` and `Event::PaneOutput` are the two `neotmux_proto` already has, the one `neotmux-proto/src/event.rs` defines and the one `crates/neotmux-lua/src/api/event.rs` matches on. `Tick.steps` is the one `decode_to_client` gives, at the line the `a_second_client` test reads, and the one the two tests of this step share.)

- [ ] **Step 6: The geometry test**

```rust
/// The last client to attach is the one the session is told to be: the bands B reserves
/// are the bands the session keeps, the surface it is laid out against, and A is looking
/// at the same one, which is to say the geometry A has been told is the one B's `attach`
/// said, and not the one it did.
#[test]
fn the_last_client_to_attach_is_the_one_the_session_is_told() {
    let runtime = TestRuntime::new();
    let _session = started(runtime.path());

    let mut a = raw_client(
        neotmux_proto::msg::Attach { wants_pane_output: false, ..an_attach() },
        &runtime,
    )
    .expect("a, in the session");
    let _ = frame_once(&mut a);

    let band = neotmux_proto::Reserved::horizontal(0, 1); // one row, from the bottom
    let mut b = raw_client(
        neotmux_proto::msg::Attach { reserved: band, ..an_attach() },
        &runtime,
    )
    .expect("b, the last to attach");

    // The frame B is owed is the frame the session was told: a `Bands` step on B's tick,
    // the one `attach` said, and not the one it did. The `Bands` the geometry has been
    // changed by is the one that is the shape this test is for, and the one it is.
    let tick_b = frame_once(&mut b);
    assert!(
        tick_b.steps.iter().any(|step| {
            matches!(
                step,
                neotmux_proto::msg::Step::Bands { applied, .. } if *applied == band
            )
        }),
        "the bands B reserved are the ones the session keeps: {tick_b:?}"
    );

    // And A is looking at the same surface the session was told: A's next `geometry.size`
    // is the one B's `attach` said the session is, and not the one it was before.
    let tick_a = frame_once(&mut a);
    assert_eq!(
        tick_a.geometry.size.rows,
        tick_b.geometry.size.rows,
        "both clients are told the same surface, the one B's attach said: {tick_a:?}"
    );
}
```

- [ ] **Step 7: The full suite, the two-client case the spec's test list wants**

Run: `cargo test -p neotmux --test socket && cargo test -p neotmux --test sessions && cargo test -p neotmux --test fixtures && cargo test -p neotmux --test headless`
Expected: PASS. `a_key_typed_on_one_client_is_seen_on_the_other` is the one in the spec's §10 list, the one the feature is for, and the one a reviewer looks for by name. The `a_second_client_is_attached_side_by_side_with_the_first` test is the one this task's stand on, the one Task 3 flipped, and the one it is.

- [ ] **Step 8: Commit**

```bash
git add crates/neotmux/tests/socket.rs
git commit -m "two clients, one session: the key typed on one, the pane the other sees"
```


### Task 5: What the person reads

**Files:**
- Modify: `README.md`
- Modify: `docs/plugins.md`
- Modify: `CHANGELOG.md`

**Interfaces:** Consumes Tasks 1–4 landed; produces the words.

- [ ] **Step 1: README**

In the paragraph the named-sessions plan rewrote (the one that ends "Two clients on one session at once is still out."), replace that closing sentence with:

```
Two terminals can be in one session at once: a key typed on either reaches the same
pane, and each keeps its own copy of the plugins' state — which is to say, its own
copy mode, its own bands, its own floats — and the one it does not keep is the
session's.
```

- [ ] **Step 2: `docs/plugins.md`**

One paragraph after the §"What a detach does not carry" section (the one the detached/attached spec put there), or next to `neotmux.detach()` on line 635 with its two neighbours:

```
**Two clients in one session.** The copy mode one has is not the one the other has,
because the copy mode is the client's and not the session's; the key that one types
and its copy mode does not consume is the key the shell on the other one sees, because
that key is the session's. A band one plugin reserves is the session's surface it
reserves it from, not a row the other client's plugins did not ask for — the last
client to attach is the one the session was told to be, and the one that is not is
the one that looks at it.
```

- [ ] **Step 3: `CHANGELOG.md`**

Under `## Unreleased`:

```
### Two clients in one session

`neotmux work`, in two terminals at once. A key typed on one is the key the other
sees, the session's and the pane's rather than the terminal's, and a plugin on one
terminal can be in a mode the other is not, the same way one can be in a mode and
the shell not be. The one that goes costs the other nothing, and the session the two
of them are in goes on until its last shell exits or the last of them calls
`neotmux.quit()`, which is to say, as long as a plugin on either of the two is still
asking for what it asked for — the OR, and not the last word, which is what it used
to be, and what "the last tock wins" would have made it.
```

- [ ] **Step 4: Whole-suite**

Run: `make check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/plugins.md CHANGELOG.md
git commit -m "the words for the two terminals: the key is the session's, the copy mode is the client's"
```

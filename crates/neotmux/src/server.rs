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
use neotmux_proto::msg::{Attach, Done, End, ToClient, ToServer, Tock};
use neotmux_proto::wire;
use neotmux_proto::{DrawCmd, Level, Notice};

use neotmux_term::wakeup::{Waker, Wakeup, wakeup};

use crate::signals::ShutdownSignal;
use neotmux::link::Link;
use neotmux::logging;
use neotmux::reads;
use neotmux::session::Apply;
use neotmux::session::Session;

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
        if frame(
            &mut session,
            &mut clients,
            listener,
            &wakeup,
            &waker,
            shutdown,
        )? {
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
        if ready == crate::tty::Ready::Idle && give_up.is_some_and(|at| Instant::now() >= at) {
            return Ok(false);
        }
        if let Some(session) = session.as_mut() {
            // Only once the poll has said so: the nudge is a pipe, and reading an empty
            // one is a wait with nothing to end it.
            if ready == crate::tty::Ready::Output {
                wakeup.clear();
            }
            if session.idle()? {
                return Ok(false);
            }
        }
        // A ready client, attached: it is the new client, the one who sets the wants-OR
        // (the other clients are none here). A connection that was greeted but never
        // attached is dropped — it may not end anybody's session, the rule the old
        // `attached` kept — and the server goes back to waiting.
        if ready == crate::tty::Ready::Input
            && let Some(stream) = taken(listener)?
            && let Some(client) = attach_new(stream, session, waker)?
        {
            let wants = clients.iter().any(|c| c.wants) || client.wants;
            if let Some(session) = session.as_mut() {
                if wants {
                    session.wants_output(true);
                } else {
                    session.forget_pane_output();
                }
            }
            clients.push(client);
            return Ok(true);
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
    let mut link = match Link::open(stream) {
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
        Some(session) => session.adopt(&attach)?,
        None => {
            // Assign through the parameter: a fresh `let session` would shadow the
            // `&mut Option<Session>` with an owned `Session` and `*session` could not
            // dereference it.
            *session = Some(Session::new(&attach, waker.clone())?);
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

/// The one wait the frame is built over: the clients being waited on, in the order they
/// attached, and the panes. One place builds the slot list for both loops, so a ready `i`
/// always lands on the slot the `slots` it is read against thinks it is — the one `n`
/// and the one list are what keep the two aligned when a client goes away mid-frame.
///
/// `BorrowedFd` is `Copy`, so the fd list is laid out on the stack: every pre-filled slot
/// is a real fd (the panes'), and the loop never makes a heap round between itself and
/// `poll`. That is how the one-process loop waited, and the budget test is the one that
/// is sensitive to it under load — the profile of the frame under it shows the heap and
/// the clock, not the wait, is where it spends its time.
fn wait_over(
    clients: &[Attached],
    n: usize,
    wakeup: &Wakeup,
    deadline: Instant,
) -> std::io::Result<crate::tty::Any> {
    const MAX_ON_STACK: usize = 32;
    if n.saturating_add(1) <= MAX_ON_STACK {
        let mut fds = [wakeup.as_fd(); MAX_ON_STACK];
        for (slot, client) in fds.iter_mut().zip(clients.iter()) {
            *slot = client.link.as_fd();
        }
        return crate::tty::wait_any(&fds[..n + 1], deadline);
    }
    let fds: Vec<std::os::fd::BorrowedFd> = clients
        .iter()
        .take(n)
        .map(|client| client.link.as_fd())
        .chain(std::iter::once(wakeup.as_fd()))
        .collect();
    crate::tty::wait_any(&fds, deadline)
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
    // the clients attached, and the panes. The tock is read off the slot of the client
    // it is, and the panes owe one to nobody.
    enum Slot {
        Client(usize),
        Panes,
    }
    // The door is looked at once a frame rather than polled on every wait: a client that
    // arrived while the previous frame was still running is adopted here, before the
    // tick, so a second client's greeting never shares the window the frame is waiting
    // for its tock in. The old loop did the same once-a-frame door-look (turning its find
    // away, `BUSY`); this one adopts the one it finds, which is the whole point of a
    // second client looking at the same session.
    if let Some(stream) = taken(listener)?
        && let Some(client) = attach_new(stream, session, waker)?
    {
        let wants = clients.iter().any(|c| c.wants) || client.wants;
        if let Some(session) = session.as_mut() {
            if wants {
                session.wants_output(true);
            } else {
                session.forget_pane_output();
            }
        }
        clients.push(client);
    }
    let n = clients.len();
    let mut slots: Vec<Slot> = (0..n).map(Slot::Client).collect();
    slots.push(Slot::Panes);

    // The tick, once, shown to every client the frame is for. A client that cannot read
    // it is a client that has gone, the rule the server has kept since M3, per
    // connection.
    // The tick, once, for every client the frame is for. `tick` is owned (it does not
    // borrow the session), so the `&mut` the tick holds is scoped to this block and the
    // `session` parameter stays free for the door arm below, which adopts a client into
    // the very same `Option`.
    let tick = {
        let Some(session) = session.as_mut() else {
            clients.retain(|client| !client.gone);
            return Ok(false);
        };
        session.tick()?
    };
    let mut expected: Vec<usize> = Vec::new();
    // The old server sent the tick with only `clients: &mut Vec<Attached>` held, and so
    // does this one: the tick is a value the session already gave, not a borrow on it.
    for (i, client) in clients.iter_mut().enumerate() {
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
        // Laid over the first `n` clients on this pass and the panes; the door is not in
        // the wait, it is looked at once at the top of the frame. The wait's borrows end
        // before the arm that answers a client takes them.
        let ready = wait_over(clients, n, wakeup, Instant::now() + TOCK)?;
        match ready {
            crate::tty::Any::Ready(i) => match slots[i] {
                Slot::Client(i) => {
                    if !clients[i].link.fill()? {
                        clients[i].gone = true;
                        continue 'collect;
                    }
                    while let Some(message) = clients[i].link.take()? {
                        match wire::decode_to_server(&message) {
                            Ok(ToServer::Tock(tock)) if tocks[i].is_none() => {
                                // The client's own question, before the tock is handed over:
                                // the frame it is for has to know whether it is one to show
                                // it with.
                                clients[i].overlay_dirty = tock.overlay != clients[i].last_overlay;
                                clients[i].notices_dirty = tock.notices != clients[i].last_notices;
                                clients[i].wants = tock.wants_pane_output;
                                tocks[i] = Some(tock);
                            }
                            Ok(ToServer::Key(key)) => {
                                let Some(session) = session.as_mut() else {
                                    break 'collect;
                                };
                                if session.key(key).is_err() {
                                    clients[i].gone = true;
                                }
                            }
                            Ok(ToServer::Resize(size)) => {
                                let Some(session) = session.as_mut() else {
                                    break 'collect;
                                };
                                if session.resize(size).is_err() {
                                    clients[i].gone = true;
                                }
                            }
                            Ok(ToServer::Read(read)) => {
                                let Some(session) = session.as_mut() else {
                                    break 'collect;
                                };
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
                }
                Slot::Panes => {
                    // A shell that wrote is not a tock: the panes are drained to keep the
                    // pipe from filling, and the frame goes on, because the tocks are the
                    // frame. The old loop broke on this in its post-done idle, not in
                    // its tock wait, which is where this one is.
                    wakeup.clear();
                }
            },
            crate::tty::Any::HangUp(i) => {
                if let Slot::Client(i) = slots[i] {
                    clients[i].gone = true;
                }
            }
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
        let Some(tock) = tocks[i].as_ref() else {
            continue;
        };
        let Some(session) = session.as_mut() else {
            continue;
        };
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
    let needs_session = session.as_mut().is_some();
    let _ = needs_session;
    if let Some(session) = session.as_mut() {
        let session_dirty = session.is_dirty();
        let force = matches!(ended, Some(End::Eof));
        for &i in &expected {
            let client = &mut clients[i];
            if client.gone {
                continue;
            }
            let Some(tock) = tocks[i].as_ref() else {
                continue;
            };
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
                // `frame_ref` is a borrow; `Done` wants a reference to it, not the value.
                frame: frame_ref.as_ref(),
                end,
            };
            if client
                .link
                .send(&wire::encode_to_client(&ToClient::Done(&done)))
                .is_err()
            {
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
    // and not one of the two it is not; a false OR is a `forget`, not `wants_output(false)`.
    clients.retain(|client| !client.gone);
    if let Some(session) = session.as_mut() {
        if clients.iter().any(|client| client.wants) {
            session.wants_output(true);
        } else {
            session.forget_pane_output();
        }
    }

    if ended.is_some() {
        return Ok(true);
    }

    // The gone were compacted out above, so the count this loop waits over is the clients
    // still in hand: a stale one would put a ready door on a client slot and index a
    // client that is no longer there.
    let n = clients.len();

    // The rest of the frame's time belongs to whatever the clients have to say and to
    // the panes: a key reaches its shell the moment it arrives, and the panes' output
    // is the cue to start the next frame rather than to wait the deadline out.
    let frame_end = Instant::now() + FRAME;
    // One slot per client still here, then the panes: the same count as the `wait_over`
    // the loop waits over, so a ready `i` lands on the right slot.
    let mut slots: Vec<Slot> = (0..n).map(Slot::Client).collect();
    slots.push(Slot::Panes);
    loop {
        if shutdown.raised().is_some() {
            return Ok(true);
        }
        if Instant::now() >= frame_end {
            break;
        }
        let ready = wait_over(clients, n, wakeup, frame_end)?;
        match ready {
            crate::tty::Any::Ready(i) => match slots[i] {
                Slot::Client(i) => {
                    if !clients[i].link.fill()? {
                        clients[i].gone = true;
                        continue;
                    }
                    while let Some(message) = clients[i].link.take()? {
                        match wire::decode_to_server(&message) {
                            Ok(ToServer::Key(key)) => {
                                // One bad client is that client's doing and costs its
                                // connection, not the frame, the rule the tock wait keeps.
                                let Some(session) = session.as_mut() else {
                                    continue;
                                };
                                if session.key(key).is_err() {
                                    clients[i].gone = true;
                                }
                            }
                            Ok(ToServer::Resize(size)) => {
                                let Some(session) = session.as_mut() else {
                                    continue;
                                };
                                if session.resize(size).is_err() {
                                    clients[i].gone = true;
                                }
                            }
                            Ok(ToServer::Read(read)) => {
                                if let Some(session) = session.as_mut() {
                                    let reading = reads::answer(session.terminals().as_ref(), read);
                                    if clients[i]
                                        .link
                                        .send(&wire::encode_to_client(&ToClient::Reading(&reading)))
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
                }
                Slot::Panes => {
                    wakeup.clear();
                    break;
                }
            },
            crate::tty::Any::HangUp(i) => {
                if let Slot::Client(i) = slots[i] {
                    clients[i].gone = true;
                }
            }
            crate::tty::Any::Idle => {}
        }
    }
    clients.retain(|client| !client.gone);
    if let Some(session) = session.as_mut() {
        if clients.iter().any(|client| client.wants) {
            session.wants_output(true);
        } else {
            session.forget_pane_output();
        }
    }
    Ok(false)
}

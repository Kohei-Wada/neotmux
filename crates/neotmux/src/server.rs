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

/// How long a connection has to say what it is. The same patience the greeting had when the
/// greeting was the whole of it, now covering the attach behind it too — and now spent at the
/// door rather than inside a frame.
const DOORSTEP: Duration = Duration::from_secs(5);

/// The one short wait the door is allowed, and only for a connection that has just greeted.
///
/// The attach is a reply to this end's hello — the peer cannot have sent it before it read
/// that — so at the moment the greeting lands the attach is on its way and a context switch
/// behind. Waiting that much for it is what keeps a client in the round the door took it in,
/// which is the round every one of them has been in since there was one to be in.
///
/// Spent once per connection: a peer that greets and then stalls costs this much once, and a
/// peer that says nothing at all never costs even that, because the wait is not reached until
/// something has been heard.
const GRACE: Duration = Duration::from_millis(10);

/// One slice of the tock wait. The tock wait has no deadline of its own — a client takes
/// as long as it takes, and the one-client loop had the same property, its `recv`
/// blocking until the client sent or stopped — so a client that stops sending is a
/// client whose connection stops, and the slice is only the length of one `poll` the
/// way it is, because the wait has to be askable of the clients and the panes on the
/// way rather than a `recv` a client holds forever.
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

/// What the server holds for the whole of its life, as against what one frame has: the
/// socket its clients arrive on, the pipe its panes nudge it with, the handle the panes are
/// given to do the nudging, and the signal that ends all of it. One value because every one
/// of them is wanted wherever any of them is.
struct Serving<'a> {
    listener: &'a UnixListener,
    wakeup: &'a Wakeup,
    waker: &'a Waker,
    shutdown: &'a ShutdownSignal,
}

pub fn serve(listener: &UnixListener, shutdown: &ShutdownSignal) -> anyhow::Result<()> {
    // Polled rather than blocked on, so the frame loop can answer a client instead of
    // leaving it in the backlog wondering.
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
    let mut session: Option<Session> = None;
    let mut clients: Vec<Attached> = Vec::new();
    // Connections that have arrived and not yet said what they are. They outlive the call
    // that took them in: a connection half-way through its handshake when a frame starts is
    // still there at the next door look, rather than being a frame's own problem.
    let mut pending: Vec<Pending> = Vec::new();
    let born = Instant::now();

    loop {
        if shutdown.raised().is_some() {
            return Ok(());
        }
        if clients.is_empty() {
            match await_client(&mut clients, &mut pending, &mut session, &serving, born)? {
                true => continue,
                // Over: signalled, out of its patience with no session, or the last
                // shell of a session no client is holding went.
                false => return Ok(()),
            }
        }
        if frame(&mut session, &mut clients, &mut pending, &serving)? {
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
    pending: &mut Vec<Pending>,
    session: &mut Option<Session>,
    serving: &Serving<'_>,
    born: Instant,
) -> anyhow::Result<bool> {
    // A session with a client that has left waits for ever: that is what detach means.
    // A server that never had one is not a session worth keeping, so `STARTUP` still
    // applies.
    let give_up = session.is_none().then_some(born + STARTUP);
    loop {
        if serving.shutdown.raised().is_some() {
            return Ok(false);
        }
        let frame_end = Instant::now() + FRAME;
        let ready = crate::tty::wait_on(
            std::os::fd::AsFd::as_fd(serving.listener),
            session.is_some().then(|| serving.wakeup.as_fd()),
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
                serving.wakeup.clear();
            }
            if session.idle()? {
                return Ok(false);
            }
        }
        // The door, looked at every pass rather than only on a ready listener: a connection
        // taken in on an earlier pass is still half-way through its handshake, and the poll
        // above answered for the listener, not for it. Nothing here waits.
        if door(pending, clients, session, serving)? {
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

/// A connection that has arrived and not yet said what it is.
///
/// It is not a client, and no frame ever waits on it: the door is looked at once a frame and
/// what is found there is read only as far as it has already spoken. A peer that connects and
/// then goes quiet sits here until its patience runs out, holding nothing but its own place —
/// the rule the old `attached` kept, now kept without the frame paying for it.
struct Pending {
    link: Link,
    give_up: Instant,
    /// Whether this connection has had its one short wait yet. See [`GRACE`].
    graced: bool,
}

/// What one look at a connection at the door found.
enum Doorstep {
    /// Nothing yet, or not all of it: it stays at the door.
    Waiting,
    /// It said what it is.
    Attaching(Attach),
    /// It is over: hung up, out of patience, or something that was not an attach — the one
    /// path that still says no to a connection, for the reason the old `attached` gave.
    Gone,
}

/// One look at one connection at the door, which never waits.
///
/// Every read here happens where a poll has just said there is something to read, and only as
/// far as that — which is the whole of C1: a handshake run inline is a handshake every other
/// client's shell is waiting on. Within one look it goes as far as the connection has already
/// spoken, so the ordinary client, whose hello and attach are written one after the other, is
/// adopted by the same door look that took it in, the way the inline handshake always did —
/// and a frame the door is inside of is not one a connection can be a step behind.
///
/// The steps are capped because the door is inside a frame: a peer trickling bytes that are
/// not yet a message is one the next look carries on with, not one this frame keeps reading.
fn doorstep(pending: &mut Pending) -> Doorstep {
    /// How many times one look goes round before leaving the rest to the next one. Two is the
    /// ordinary client — its hello, then its attach; the rest is room for a peer whose writes
    /// landed in more pieces than it made them in.
    const STEPS: usize = 8;

    for _ in 0..STEPS {
        // Nothing here waits, with the one exception [`GRACE`] is: a connection that has just
        // greeted is a connection whose attach is already on the wire.
        let until = if pending.link.is_greeted() && !pending.graced {
            pending.graced = true;
            Instant::now() + GRACE
        } else {
            Instant::now()
        };
        let ready = match crate::tty::wait_any(&[pending.link.as_fd()], until) {
            Ok(crate::tty::Any::Ready(_)) => true,
            Ok(crate::tty::Any::Idle) => false,
            Ok(crate::tty::Any::HangUp(_)) => return Doorstep::Gone,
            Err(err) => {
                logging::log(
                    Level::Warn,
                    "neotmux-server",
                    &format!("a connection at the door could not be waited on: {err}"),
                );
                return Doorstep::Gone;
            }
        };
        if !ready {
            break;
        }
        if !pending.link.is_greeted() {
            match pending.link.greeted() {
                Ok(_) => continue,
                // A client built against another protocol, or one that hung up mid-handshake.
                // Its own end says so; this one goes back to what it was doing.
                Err(err) => {
                    logging::log(
                        Level::Warn,
                        "neotmux-server",
                        &format!("a client could not be greeted: {err}"),
                    );
                    return Doorstep::Gone;
                }
            }
        }
        match pending.link.fill() {
            Ok(true) => {}
            Ok(false) => return Doorstep::Gone,
            Err(err) => {
                logging::log(
                    Level::Warn,
                    "neotmux-server",
                    &format!("a client's first message could not be read: {err}"),
                );
                return Doorstep::Gone;
            }
        }
        let message = match pending.link.take() {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(err) => {
                logging::log(
                    Level::Warn,
                    "neotmux-server",
                    &format!("a client's first message could not be read: {err}"),
                );
                return Doorstep::Gone;
            }
        };
        return match wire::decode_to_server(&message) {
            Ok(ToServer::Attach(attach)) => Doorstep::Attaching(attach),
            Ok(other) => {
                logging::log(
                    Level::Warn,
                    "neotmux-server",
                    &format!("a client's first message was {other:?} rather than an attach"),
                );
                Doorstep::Gone
            }
            Err(err) => {
                logging::log(
                    Level::Warn,
                    "neotmux-server",
                    &format!("a client's first message could not be decoded: {err}"),
                );
                Doorstep::Gone
            }
        };
    }

    if Instant::now() >= pending.give_up {
        logging::log(
            Level::Warn,
            "neotmux-server",
            "a connection said nothing for long enough to be nobody",
        );
        return Doorstep::Gone;
    }
    Doorstep::Waiting
}

/// Everything at the door, looked at once and never listened to: whatever has arrived on the
/// listener is taken and put there, everything already there is stepped, and whichever of them
/// has said what it is becomes a client — a session adopting it, or, where there is no session
/// yet, becoming one.
///
/// `true` is somebody having been adopted. The `wants_pane_output` OR is taken here because
/// here is where the set of attached clients changes; a false OR is a `forget`, not a
/// `wants_output(false)`, so the stale steps go with it.
fn door(
    pending: &mut Vec<Pending>,
    clients: &mut Vec<Attached>,
    session: &mut Option<Session>,
    serving: &Serving<'_>,
) -> anyhow::Result<bool> {
    while let Some(stream) = taken(serving.listener)? {
        match Link::greet(stream) {
            Ok(link) => pending.push(Pending {
                link,
                give_up: Instant::now() + DOORSTEP,
                graced: false,
            }),
            Err(err) => logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("a client could not be greeted: {err}"),
            ),
        }
    }

    let mut adopted = false;
    let mut waiting: Vec<Pending> = Vec::new();
    for mut one in std::mem::take(pending) {
        match doorstep(&mut one) {
            Doorstep::Waiting => waiting.push(one),
            Doorstep::Gone => {}
            Doorstep::Attaching(attach) => {
                clients.push(admit(one.link, &attach, session, serving.waker)?);
                adopted = true;
            }
        }
    }
    *pending = waiting;

    if adopted && let Some(session) = session.as_mut() {
        if clients.iter().any(|client| client.wants) {
            session.wants_output(true);
        } else {
            session.forget_pane_output();
        }
    }
    Ok(adopted)
}

/// The client a session adopts, out of the attach it sent — and, where there is no session,
/// the session it is the first of.
fn admit(
    link: Link,
    attach: &Attach,
    session: &mut Option<Session>,
    waker: &Waker,
) -> anyhow::Result<Attached> {
    match session.as_mut() {
        Some(session) => session.adopt(attach)?,
        None => {
            // Assign through the parameter: a fresh `let session` would shadow the
            // `&mut Option<Session>` with an owned `Session` and `*session` could not
            // dereference it.
            *session = Some(Session::new(attach, waker.clone())?);
        }
    }
    Ok(Attached {
        link,
        wants: attach.wants_pane_output,
        last_overlay: Vec::new(),
        last_notices: Vec::new(),
        first: true,
        gone: false,
        overlay_dirty: false,
        notices_dirty: false,
    })
}

/// The one wait the frame is built over: the clients still being waited on, in the order
/// they attached, and the panes. `live` is the slot list — one entry per client the wait is
/// for, holding that client's index — and the panes are the slot after the last of them, so
/// a ready slot always lands on the client `live` says it is.
///
/// A client that has gone is left out of it. `POLLHUP` is level-triggered, so a hung-up fd
/// left in the set comes back ready on every pass: the wait would return at once, for ever,
/// while another client's tock is still owed — a whole core burnt at the moment one client
/// has died and the rest are being served.
///
/// `BorrowedFd` is `Copy`, so the fd list is laid out on the stack: every pre-filled slot
/// is a real fd (the panes'), and the loop never makes a heap round between itself and
/// `poll`. That is how the one-process loop waited, and the budget test is the one that
/// is sensitive to it under load — the profile of the frame under it shows the heap and
/// the clock, not the wait, is where it spends its time.
fn wait_over(
    clients: &[Attached],
    live: &[usize],
    wakeup: &Wakeup,
    deadline: Instant,
) -> std::io::Result<crate::tty::Any> {
    const MAX_ON_STACK: usize = 32;
    let n = live.len();
    if n.saturating_add(1) <= MAX_ON_STACK {
        let mut fds = [wakeup.as_fd(); MAX_ON_STACK];
        for (slot, &i) in fds.iter_mut().zip(live.iter()) {
            *slot = clients[i].link.as_fd();
        }
        return crate::tty::wait_any(&fds[..n + 1], deadline);
    }
    let fds: Vec<std::os::fd::BorrowedFd> = live
        .iter()
        .map(|&i| clients[i].link.as_fd())
        .chain(std::iter::once(wakeup.as_fd()))
        .collect();
    crate::tty::wait_any(&fds, deadline)
}

/// Whether this client's link is still worth reading, having read whatever had arrived.
///
/// An I/O error on one link is that client's own doing — a length prefix nobody could
/// mean, a read that failed on its socket and no other — and costs its connection and
/// nothing else, which is the deal the undecodable message already got. Letting it out
/// of here would end the session, and with it every other client's shells.
fn still_reading(client: &mut Attached) -> bool {
    match client.link.fill() {
        Ok(true) => true,
        Ok(false) => {
            client.gone = true;
            false
        }
        Err(err) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("an attached client's link could not be read: {err}"),
            );
            client.gone = true;
            false
        }
    }
}

/// The next whole message this client has already sent, or `None` for "not one" — which
/// is either that the rest of it has not arrived or that the link is over, and `gone` is
/// what tells those apart.
fn next_message(client: &mut Attached) -> Option<Vec<u8>> {
    match client.link.take() {
        Ok(message) => message,
        Err(err) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("an attached client announced a message that would not be one: {err}"),
            );
            client.gone = true;
            None
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
    pending: &mut Vec<Pending>,
    serving: &Serving<'_>,
) -> anyhow::Result<bool> {
    // The door is looked at once a frame rather than polled on every wait: a client that
    // arrived while the previous frame was still running is adopted here, before the
    // tick, so a second client's greeting never shares the window the frame is waiting
    // for its tock in. The old loop did the same once-a-frame door-look (turning its find
    // away, `BUSY`); this one adopts the one it finds, which is the whole point of a
    // second client looking at the same session.
    door(pending, clients, session, serving)?;
    // The slots the frame waits over: one per client it is still waiting on, holding that
    // client's index, and the panes after the last of them. Refilled on every pass rather
    // than built once — a client that goes mid-frame leaves the wait on the next pass, and
    // its slot cannot be the slot a later ready is read against.
    let mut live: Vec<usize> = Vec::with_capacity(clients.len() + 1);

    // The tick, once, shown to every client the frame is for, and a client that cannot
    // read it is a client that has gone, the rule the server has kept since M3, per
    // connection. `tick` is owned (it does not borrow the session), so the `&mut` it
    // holds is scoped to this block and the session stays free for the waits below.
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
        if serving.shutdown.raised().is_some() {
            return Ok(true);
        }
        if !expected
            .iter()
            .any(|&i| !clients[i].gone && tocks[i].is_none())
        {
            break 'collect;
        }
        // Laid over the clients still here on this pass and the panes; the door is not in
        // the wait, it is looked at once at the top of the frame. The wait's borrows end
        // before the arm that answers a client takes them.
        live.clear();
        live.extend((0..clients.len()).filter(|&i| !clients[i].gone));
        let ready = wait_over(clients, &live, serving.wakeup, Instant::now() + TOCK)?;
        match ready {
            crate::tty::Any::Ready(slot) => match live.get(slot).copied() {
                Some(i) => {
                    if !still_reading(&mut clients[i]) {
                        continue 'collect;
                    }
                    while let Some(message) = next_message(&mut clients[i]) {
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
                        // A client dropped for what it sent does not go on typing into the
                        // shared pane: whatever else is in its inbox goes with it.
                        if clients[i].gone {
                            break;
                        }
                    }
                }
                // The slot past the last client is the panes. A shell that wrote is not a
                // tock: the panes are drained to keep the pipe from filling, and the frame
                // goes on, because the tocks are the frame. The old loop broke on this in
                // its post-done idle, not in its tock wait, which is where this one is.
                None => serving.wakeup.clear(),
            },
            crate::tty::Any::HangUp(slot) => {
                if let Some(&i) = live.get(slot) {
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
    // Every client that asked to leave, not just one: two plugins can call `neotmux.detach()`
    // in the same frame, and the one whose ask was overwritten would be told `end: None` and
    // stay attached with nothing to say why.
    let mut detachers: Vec<usize> = Vec::new();
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
                End::Detach => detachers.push(i),
                other => ended = Some(other),
            }
        }
    }

    // The picture, per client, because the picture is per client. The session's dirt is
    // the same for all of them and is read before the first render, because a render is
    // what clears it; the overlay and the notices are each one's own.
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
            let end = if detachers.contains(&i) {
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
            if detachers.contains(&i) {
                // The one that is detaching is out, and its connection is about to close
                // for the reason it asked to leave: drop it here rather than at the next
                // frame's first send.
                client.gone = true;
            }
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

    // The rest of the frame's time belongs to whatever the clients have to say and to
    // the panes: a key reaches its shell the moment it arrives, and the panes' output
    // is the cue to start the next frame rather than to wait the deadline out.
    let frame_end = Instant::now() + FRAME;
    loop {
        if serving.shutdown.raised().is_some() {
            return Ok(true);
        }
        if Instant::now() >= frame_end {
            break;
        }
        live.clear();
        live.extend((0..clients.len()).filter(|&i| !clients[i].gone));
        let ready = wait_over(clients, &live, serving.wakeup, frame_end)?;
        match ready {
            crate::tty::Any::Ready(slot) => match live.get(slot).copied() {
                Some(i) => {
                    if !still_reading(&mut clients[i]) {
                        continue;
                    }
                    while let Some(message) = next_message(&mut clients[i]) {
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
                        // A client dropped for what it sent does not go on typing into the
                        // shared pane: whatever else is in its inbox goes with it.
                        if clients[i].gone {
                            break;
                        }
                    }
                }
                // The panes: their output is the cue to start the next frame.
                None => {
                    serving.wakeup.clear();
                    break;
                }
            },
            crate::tty::Any::HangUp(slot) => {
                if let Some(&i) = live.get(slot) {
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

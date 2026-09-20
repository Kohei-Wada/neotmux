//! The process that holds the session.
//!
//! It has no terminal: it owns the panes, their children, the tree they are laid out in and
//! the terminal state behind each of them, and it never looks at any of it. Every frame is a
//! conversation with the client — what happened, what the plugins asked for, what to show —
//! and the pane reads a plugin makes are answered in the one window where the server is
//! already waiting for that client to speak.
//!
//! Its life is the session's: when the client goes away the session is kept until its last
//! shell exits, still reading its children, so a disconnect is not a crash. A client that
//! arrives afterwards is shown that same session rather than given a new one; a client
//! arriving while one is already attached is refused with a reason rather than left
//! hanging in the backlog. Naming which client gets to look at a session at a time is as
//! far as this file goes — that a session sits waiting at all is the rest of it.

use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use neotmux_proto::Level;
use neotmux_proto::msg::{Done, ToClient, ToServer};
use neotmux_proto::wire;
use std::os::fd::AsFd as _;

use neotmux_term::wakeup::{Waker, Wakeup, wakeup};

use neotmux::link::Link;
use neotmux::logging;
use neotmux::reads;
use neotmux::session::Session;

use crate::signals::ShutdownSignal;

/// The frame's cadence, and so the closest together two frames can be. The same number the
/// one-process loop used, for the same reason: it is a property of the loop rather than of
/// whatever the wait happens to do.
const FRAME: Duration = Duration::from_millis(16);

/// How long a server with no session waits for the client that started it. It is the
/// client's own patience: past that, nobody is coming and there is nothing to serve.
const STARTUP: Duration = Duration::from_secs(10);

const BUSY: &str = "this session already has a client attached";

pub fn serve(listener: &UnixListener, shutdown: &ShutdownSignal) -> anyhow::Result<()> {
    // Polled rather than blocked on, so the frame loop can answer a second client instead of
    // leaving it in the backlog wondering.
    listener
        .set_nonblocking(true)
        .context("make the socket pollable")?;
    let (wakeup, waker) = wakeup().context("open the panes' wakeup")?;
    let mut session: Option<Session> = None;
    let born = Instant::now();

    loop {
        let Some(stream) = accept(listener, session.as_mut(), &wakeup, shutdown, born)? else {
            return Ok(());
        };
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
                continue;
            }
        };
        let mut running = match attached(&mut link, &waker, &mut session)? {
            Some(started) => started,
            None => continue,
        };
        let over = converse(&mut link, &mut running, listener, &wakeup, shutdown)?;
        if over {
            return Ok(());
        }
        // The client went, the session did not. It keeps reading its children for as long as
        // it lives, and the loop goes back to waiting for somebody to look at it.
        running.forget_pane_output();
        session = Some(running);
    }
}

/// Waits for a client, running the session in the meantime so its children are not left
/// talking into a pipe nobody empties. `None` means there will not be one: startup timed
/// out, the session ended, or a signal arrived.
fn accept(
    listener: &UnixListener,
    mut session: Option<&mut Session>,
    wakeup: &Wakeup,
    shutdown: &ShutdownSignal,
    born: Instant,
) -> anyhow::Result<Option<UnixStream>> {
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
        if let Some(session) = session.as_deref_mut() {
            // Only once the poll has said so: the nudge is a pipe, and reading an empty one
            // is a wait with nothing to end it.
            if ready == crate::tty::Ready::Output {
                wakeup.clear();
            }
            if session.idle()? {
                return Ok(None);
            }
        }
        if ready == crate::tty::Ready::Input
            && let Some(stream) = taken(listener)?
        {
            return Ok(Some(stream));
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

/// Turns away a client the session is not free to take, without a conversation: the
/// greeting this end owes it, the reason, and the door.
///
/// Written straight to the socket rather than through a [`Link`], because a [`Link`] reads,
/// and this is called from inside a frame that has a deadline. Both writes are small enough
/// that a socket nobody has written to yet takes them without waiting.
fn turn_away(mut stream: UnixStream, why: &str) {
    use std::io::Write as _;
    let refusal = wire::encode_to_client(&ToClient::Refused(why));
    let _ = stream.write_all(&wire::encode_hello());
    let _ = stream.write_all(&u32::try_from(refusal.len()).unwrap_or(0).to_le_bytes());
    let _ = stream.write_all(&refusal);
    let _ = stream.flush();
}

/// The client's first message, applied. With a session in hand this adopts it; without one
/// it builds it. `None` means no session came of this connection — the client went away,
/// or said something that was not a usable `Attach` — and `session` is left exactly as it
/// was found.
///
/// Nothing a client does to its own connection may end somebody's session: `session` is
/// only taken once an `Attach` has actually been read and decoded, and every failure that
/// is the connection's own — a hangup, an unreadable message, a message that is not an
/// `Attach` — returns `Ok(None)` rather than propagating an error. An error from here
/// still ends `serve`, so only failures that are genuinely the server's own — `adopt` or
/// `Session::new` failing to lay out or spawn a shell — are left as `?`.
fn attached(
    link: &mut Link,
    waker: &Waker,
    session: &mut Option<Session>,
) -> anyhow::Result<Option<Session>> {
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
    let attach = match wire::decode_to_server(&message) {
        Ok(ToServer::Attach(attach)) => attach,
        Ok(other) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("a client's first message was {other:?} rather than an attach"),
            );
            return Ok(None);
        }
        Err(err) => {
            logging::log(
                Level::Warn,
                "neotmux-server",
                &format!("a client's first message could not be decoded: {err}"),
            );
            return Ok(None);
        }
    };
    Ok(Some(match session.take() {
        Some(mut session) => {
            session.adopt(&attach)?;
            session
        }
        None => Session::new(&attach, waker.clone())?,
    }))
}

/// One client's whole conversation. `true` means the session is over and so is this process.
fn converse(
    link: &mut Link,
    session: &mut Session,
    listener: &UnixListener,
    wakeup: &Wakeup,
    shutdown: &ShutdownSignal,
) -> anyhow::Result<bool> {
    loop {
        if shutdown.raised().is_some() {
            return Ok(true);
        }
        let frame_end = Instant::now() + FRAME;
        // Turned away here rather than left in the backlog: a second neotmux should say why
        // it is not getting this session, not sit waiting for one.
        if let Some(waiting) = taken(listener)? {
            turn_away(waiting, BUSY);
        }

        let tick = session.tick()?;
        if send(link, &ToClient::Tick(&tick))?.is_none() {
            return Ok(false);
        }
        let Some(tock) = await_tock(link, session, shutdown)? else {
            return Ok(false);
        };

        let mut pass = session.advance(&tock)?;
        let end = pass.end;
        let done = Done {
            steps: std::mem::take(&mut pass.steps),
            frame: pass.frame.as_ref(),
            end,
        };
        let encoded = wire::encode_to_client(&ToClient::Done(&done));
        drop(done);
        drop(pass);
        let delivered = link.send(&encoded).is_ok();
        // Quit and Eof are both the session ending, and the session is all this process is
        // for.
        if end.is_some() {
            return Ok(true);
        }
        if !delivered {
            return Ok(false);
        }

        // The rest of the frame's time belongs to whatever the client has to say and to the
        // panes: a key reaches its shell the moment it arrives, and a pane's output is the
        // cue to start the next frame rather than to wait the deadline out.
        loop {
            if shutdown.raised().is_some() {
                return Ok(true);
            }
            match crate::tty::wait_on(link.as_fd(), Some(wakeup.as_fd()), frame_end)
                .context("wait for the client")?
            {
                crate::tty::Ready::HangUp => return Ok(false),
                crate::tty::Ready::Input => {
                    if !link.fill()? {
                        return Ok(false);
                    }
                    if !apply_pending(link, session)? {
                        return Ok(false);
                    }
                }
                crate::tty::Ready::Output => {
                    wakeup.clear();
                    break;
                }
                crate::tty::Ready::Idle => {}
            }
            if Instant::now() >= frame_end {
                break;
            }
        }
    }
}

/// Everything the client has already sent, applied. `false` means it sent something only the
/// tick loop should see, or went away.
fn apply_pending(link: &mut Link, session: &mut Session) -> anyhow::Result<bool> {
    while let Some(message) = link.take()? {
        match wire::decode_to_server(&message).context("decode a client message")? {
            ToServer::Key(key) => session.key(key)?,
            ToServer::Resize(size) => session.resize(size)?,
            ToServer::Read(read) => {
                let reading = reads::answer(session.terminals().as_ref(), read);
                if link
                    .send(&wire::encode_to_client(&ToClient::Reading(&reading)))
                    .is_err()
                {
                    return Ok(false);
                }
            }
            // A reply to a tick this loop is not inside, or a second attach. Neither is
            // something the session can act on out here.
            ToServer::Tock(_) | ToServer::Attach(_) => {}
        }
    }
    Ok(true)
}

/// Waits for the client to finish its tick, answering the pane reads it makes along the way.
/// This is the one window in which a read can arrive, which is why a read never has to be
/// picked out of a stream of frames at either end.
fn await_tock(
    link: &mut Link,
    session: &mut Session,
    shutdown: &ShutdownSignal,
) -> anyhow::Result<Option<neotmux_proto::msg::Tock>> {
    loop {
        if shutdown.raised().is_some() {
            return Ok(None);
        }
        let Some(message) = link.recv()? else {
            return Ok(None);
        };
        match wire::decode_to_server(&message).context("decode a client message")? {
            ToServer::Tock(tock) => return Ok(Some(tock)),
            ToServer::Key(key) => session.key(key)?,
            ToServer::Resize(size) => session.resize(size)?,
            ToServer::Read(read) => {
                let reading = reads::answer(session.terminals().as_ref(), read);
                if link
                    .send(&wire::encode_to_client(&ToClient::Reading(&reading)))
                    .is_err()
                {
                    return Ok(None);
                }
            }
            ToServer::Attach(_) => {}
        }
    }
}

/// `None` is the client having gone away, which is not an error: a terminal closing is how a
/// session is usually left.
fn send(link: &mut Link, msg: &ToClient<'_>) -> anyhow::Result<Option<()>> {
    Ok(link.send(&wire::encode_to_client(msg)).ok())
}

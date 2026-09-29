use std::os::fd::AsFd as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::wire::{Done, End, ToClient, ToServer};
use neotmux_proto::{Level, SessionId, wire};
use neotmux_pty::PaneTerminals;
use neotmux_pty::child::{Waker, Wakeup, wakeup};
use neotmux_sys::sock::{self, Lock, Paths};
use neotmux_sys::{Link, ShutdownSignal, logging};

use crate::doorstep::{Pending, await_client, door};
use crate::sessions::{end_session, rename_session, start_session, switch_clients};
use crate::{Attached, Clip, Pass, Plugins, RenameFailure, ServerOp, Sessions, StartFailure};

/// The frame's cadence, and so the closest together two frames can be. The same number
/// the one-process loop used, for the same reason: it is a property of the loop rather
/// than of whatever the wait happens to do.
pub(crate) const FRAME: Duration = Duration::from_millis(16);

/// How long a server with no session waits for the client that started it. It is the
/// client's own patience: past that, nobody is coming and there is nothing to serve.
pub(crate) const STARTUP: Duration = Duration::from_secs(10);

/// How long a connection has to say what it is. The same patience the greeting had when the
/// greeting was the whole of it, now covering the attach behind it too — and now spent at the
/// door rather than inside a frame.
pub(crate) const DOORSTEP: Duration = Duration::from_secs(5);

/// The server, as `--server` runs it: the lock that says this process is the one, the
/// socket it binds, `serve` for as long as there is a session, and the taking of both
/// away again.
///
/// It stands down quietly rather than complaining when somebody else got there first: two
/// clients starting at the same instant will both have tried, and the loser has nothing to
/// report — its client is about to connect to the winner.
pub fn run() -> anyhow::Result<()> {
    let paths = Paths::resolve(&|key| std::env::var_os(key))?;
    let Some(_lock) = Lock::take(&paths)? else {
        return Ok(());
    };
    let Some(listener) = sock::bind(&paths)? else {
        return Ok(());
    };
    let shutdown = ShutdownSignal::install()?;
    let served = serve(&listener, &shutdown);
    // Taken away on the way out, so the next client does not connect to a server that is
    // gone — and only if it is still this server's, so one that outlived a replacement does
    // not take the replacement's name with it.
    sock::unbind(&paths, &listener);
    // The server's standard error goes to `/dev/null` — it has no screen to complain to —
    // so the log is the only place a failure can be read afterwards.
    if let Err(err) = &served {
        logging::log(
            Level::Error,
            "neotmux-server",
            &format!("the session ended in an error: {err:#}"),
        );
    }
    served
}

/// What the server holds for the whole of its life, as against what one frame has: the
/// socket its clients arrive on, the pipe its panes nudge it with, the handle the panes are
/// given to do the nudging, and the signal that ends all of it. One value because every one
/// of them is wanted wherever any of them is.
pub(crate) struct Serving<'a> {
    pub(crate) listener: &'a UnixListener,
    wakeup: &'a Wakeup,
    pub(crate) waker: &'a Waker,
    pub(crate) shutdown: &'a ShutdownSignal,
    /// The panes' terminal state, made before the session so the Lua layer can read it.
    pub(crate) terminals: &'a Rc<PaneTerminals>,
}

fn serve(listener: &UnixListener, shutdown: &ShutdownSignal) -> anyhow::Result<()> {
    // Polled rather than blocked on, so the frame loop can answer a client instead of
    // leaving it in the backlog wondering.
    listener
        .set_nonblocking(true)
        .context("make the socket pollable")?;
    let (wakeup, waker) = wakeup().context("open the panes' wakeup")?;
    // Before the first client and before the session: the first window's id, the bands and
    // the gutter are the Lua layer's to say, and the panes it reads are the ones the session
    // is about to be given.
    let terminals = Rc::new(PaneTerminals::new());
    let serving = Serving {
        listener,
        wakeup: &wakeup,
        waker: &waker,
        shutdown,
        terminals: &terminals,
    };

    let clip = Rc::new(Clip::new());
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
    let layers = Layers::resolve(&|key| std::env::var_os(key), exe_dir.as_deref());
    // The waker the panes' reader threads already hold: a job's pipes are drained by threads
    // of its own and nudge the same wakeup, so nothing here is added to the poll set.
    let runner = Rc::new(crate::Processes::new(waker.clone()));
    let lua = LuaSubsystem::build_with_runner(layers, terminals.clone(), clip.clone(), runner)?;
    let mut plugins = Plugins::new(lua);
    for failure in plugins.lua().run_init() {
        logging::log(Level::Warn, &failure.source, &failure.message);
    }
    // A plugin that called `neotmux.quit()` while loading has said there is to be no session.
    // The server stays up only long enough to tell whoever started it why: one that simply
    // exited would be a client waiting out its whole start timeout for a socket that never
    // answers.
    if plugins.drain_load_ops() {
        return nothing_to_serve(&serving, &plugins);
    }

    let mut sessions = Sessions::default();
    let mut clients: Vec<Attached> = Vec::new();
    // Connections that have arrived and not yet said what they are. They outlive the call
    // that took them in: a connection half-way through its handshake when a frame starts is
    // still there at the next door look, rather than being a frame's own problem.
    let mut pending: Vec<Pending> = Vec::new();
    let born = Instant::now();
    // When the last frame that pane output started began: the next one it starts waits a
    // whole `FRAME` after it.
    let mut cue: Option<Instant> = None;

    loop {
        if shutdown.raised().is_some() {
            return Ok(());
        }
        // Only ever about a server that has no session yet: past that, a frame runs whether
        // or not anybody is attached to see it.
        if !await_client(
            &mut clients,
            &mut pending,
            &mut plugins,
            &mut sessions,
            &serving,
            born,
        )? {
            return Ok(());
        }
        if frame(
            &mut sessions,
            &mut plugins,
            &clip,
            &mut clients,
            &mut pending,
            &serving,
            &mut cue,
        )? {
            return Ok(());
        }
    }
}

/// A server with nothing to serve, staying up only long enough to say so: a plugin called
/// `neotmux.quit()` while loading, so the session is over before it began.
///
/// It is said as an end and not as a refusal, because that is what it is — nothing went
/// wrong — and because the client reads this before it touches the screen: an end here is a
/// client that exits without ever entering the alternate screen.
fn nothing_to_serve(serving: &Serving<'_>, plugins: &Plugins) -> anyhow::Result<()> {
    let give_up = Instant::now() + STARTUP;
    let transcript = plugins.notifications().since(0);
    while Instant::now() < give_up {
        if serving.shutdown.raised().is_some() {
            return Ok(());
        }
        match taken(serving.listener)? {
            Some(stream) => {
                let Ok(mut link) = Link::greet(stream) else {
                    continue;
                };
                let _ = link.send(&wire::encode_to_client(&ToClient::Bye(&transcript)));
                let _ = link.send(&wire::encode_to_client(&ToClient::Done(&Done {
                    frame: None,
                    end: Some(End::Quit),
                })));
                // Held open until the client has read that and gone. A server that exited the
                // moment it wrote would have the client's own attach fail with a broken pipe,
                // and a broken pipe is not what happened here.
                let until = Instant::now() + DOORSTEP;
                while Instant::now() < until {
                    match neotmux_sys::tty::wait_any(&[link.as_fd()], until) {
                        Ok(neotmux_sys::tty::Any::Ready(_)) => match link.fill() {
                            Ok(true) => continue,
                            _ => break,
                        },
                        Ok(neotmux_sys::tty::Any::Idle) => continue,
                        _ => break,
                    }
                }
                // Whoever started this server is the one that was waiting for it; there is no
                // reason for anybody else to find it.
                return Ok(());
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    Ok(())
}

/// A client waiting on the socket, if there is one. The listener is non-blocking, so this
/// can be asked at any point of a frame without risking one.
pub(crate) fn taken(listener: &UnixListener) -> anyhow::Result<Option<UnixStream>> {
    match listener.accept() {
        Ok((stream, _)) => {
            // macOS hands the listener's O_NONBLOCK on to what it accepts; Linux does not.
            stream
                .set_nonblocking(false)
                .context("make a client's socket blocking")?;
            Ok(Some(stream))
        }
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(err).context("accept a client"),
    }
}

/// What a plugin asked of the set of sessions, applied once every session has had its frame.
/// The session whose reload wins the frame, if any of this frame's ops asked for one: the
/// first to ask, since every other reload asked by the same frame is the same config reread
/// twice over.
fn reload_requested(asked: &[(SessionId, String, ServerOp)]) -> Option<SessionId> {
    asked
        .iter()
        .find(|(_, _, op)| matches!(op, ServerOp::Reload))
        .map(|(from, _, _)| *from)
}

fn apply_server_ops(
    asked: Vec<(SessionId, String, ServerOp)>,
    plugins: &mut Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
    terminals: &Rc<PaneTerminals>,
    waker: &Waker,
) {
    let reload = reload_requested(&asked);
    for (from, tag, op) in asked {
        match op {
            ServerOp::Create { name } => {
                // At the size of the session that asked: nobody is looking at the new one
                // yet, and a surface has to be some size before a shell can open on it.
                let Some(size) = sessions.get_mut(from).map(|session| session.surface_size())
                else {
                    continue;
                };
                if let Err(err) = start_session(&name, size, plugins, sessions, terminals, waker) {
                    let said = match err {
                        StartFailure::Invalid(why) => format!("session.create refused: {why}"),
                        StartFailure::Taken => {
                            format!("session.create('{name}') refused: it already exists")
                        }
                        StartFailure::Failed(err) => {
                            format!("session.create('{name}') failed: {err:#}")
                        }
                    };
                    logging::log(Level::Error, &tag, &said);
                }
            }
            ServerOp::Switch { name } => match sessions.id_of(&name) {
                Some(to) => {
                    if let Err(err) = switch_clients(to, Some(from), clients, sessions, plugins) {
                        logging::log(
                            Level::Error,
                            &tag,
                            &format!("session.switch('{name}') failed: {err:#}"),
                        );
                    }
                }
                None => logging::log(
                    Level::Error,
                    &tag,
                    &format!("session.switch('{name}') refused: no session by that name"),
                ),
            },
            ServerOp::Rename { from, to } => {
                if let Err(err) = rename_session(&from, &to, plugins, sessions) {
                    let said = match err {
                        RenameFailure::Invalid(why) => format!("session.rename refused: {why}"),
                        RenameFailure::Taken => {
                            format!("session.rename to '{to}' refused: it already exists")
                        }
                        RenameFailure::Missing => format!(
                            "session.rename('{from}', '{to}') refused: no session by that name"
                        ),
                    };
                    logging::log(Level::Error, &tag, &said);
                }
            }
            ServerOp::Kill { name } => match sessions.id_of(&name) {
                Some(id) => end_session(id, plugins, sessions, clients),
                None => logging::log(
                    Level::Error,
                    &tag,
                    &format!("session.kill('{name}') refused: no session by that name"),
                ),
            },
            // Applied once, after the loop: however many sessions asked this frame, the
            // config is read once.
            ServerOp::Reload => {}
        }
    }
    // Last, and once however many asked: every other op this frame was raised by the old
    // code and is carried out as the old code meant it.
    if let Some(from) = reload {
        plugins.reload(sessions.get_mut(from).is_some().then_some(from));
    }
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
) -> std::io::Result<neotmux_sys::tty::Any> {
    const MAX_ON_STACK: usize = 32;
    let n = live.len();
    if n.saturating_add(1) <= MAX_ON_STACK {
        let mut fds = [wakeup.as_fd(); MAX_ON_STACK];
        for (slot, &i) in fds.iter_mut().zip(live.iter()) {
            *slot = clients[i].link.as_fd();
        }
        return neotmux_sys::tty::wait_any(&fds[..n + 1], deadline);
    }
    let fds: Vec<std::os::fd::BorrowedFd> = live
        .iter()
        .map(|&i| clients[i].link.as_fd())
        .chain(std::iter::once(wakeup.as_fd()))
        .collect();
    neotmux_sys::tty::wait_any(&fds, deadline)
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

/// One frame of the session: what happened, what the plugins made of it, what that did, and
/// the picture every attached client is shown.
///
/// It is four function calls where it used to be four messages. The Lua layer is in this
/// process now, so the tick it answers is a value and not something on a socket, and the
/// clients are only shown the result.
///
/// `true` is the server being over — the session ending on a `Quit` or its last shell — and
/// `false` is the frame being over, the session going on with however many clients are left.
fn frame(
    sessions: &mut Sessions,
    plugins: &mut Plugins,
    clip: &Clip,
    clients: &mut Vec<Attached>,
    pending: &mut Vec<Pending>,
    serving: &Serving<'_>,
    cue: &mut Option<Instant>,
) -> anyhow::Result<bool> {
    // The door is looked at once a frame rather than polled on every wait: a client that
    // arrived while the previous frame was still running is adopted here, before the tick,
    // so it is in the frame it arrived in.
    door(pending, clients, plugins, sessions, serving)?;
    clip.attached(
        clients
            .iter()
            .filter(|client| !client.gone)
            .map(|client| client.session),
    );

    if sessions.live.is_empty() {
        clients.retain(|client| !client.gone);
        return Ok(false);
    }

    // Every session, in id order, whether or not anybody is attached to it: a session nobody
    // is looking at running its plugins is what putting them here bought. There is one of
    // them today, so this is one pass.
    let ids: Vec<SessionId> = sessions.live.keys().copied().collect();
    let mut over = false;
    // Collected rather than applied in the loop: a client that switches must not be sent two
    // frames in one pass, and a session made here must not tick before it has been drawn.
    let mut asked: Vec<(SessionId, String, ServerOp)> = Vec::new();
    for id in ids {
        // A session that ends with others still live keeps its clients, which move to one of
        // them in `end_session`, so they are not told to leave.
        let alone = sessions.live.len() == 1;
        let Some(live) = sessions.get_mut(id) else {
            continue;
        };

        // The whole of what used to be four messages: the session says what happened, the Lua
        // layer answers, the session applies that, and the picture comes of it.
        let looking = clients
            .iter()
            .filter(|client| !client.gone && client.session == id)
            .count();
        plugins.lua().publish_attached(id, looking);
        let tick = live.tick()?;
        let answer = plugins.respond(id, tick);
        asked.extend(answer.server.into_iter().map(|(tag, op)| (id, tag, op)));
        let Pass { steps, frame, end } = live.advance(&answer.tock, &answer.views)?;
        let told = end.filter(|end| alone || *end == End::Detach);
        let picture = (frame.is_some() || told.is_some()).then(|| {
            // The steps are not the client's: they go to the Lua layer in this process, below.
            wire::encode_to_client(&ToClient::Done(&Done {
                frame: frame.as_ref(),
                end: told,
            }))
        });
        drop(frame);

        // This session's own yanks, whenever in the frame its plugins raised them: each is kept
        // with the session it was raised for, so another session's frame never takes it.
        let yanked: Vec<Vec<u8>> = clip
            .take(id)
            .iter()
            .map(|text| wire::encode_to_client(&ToClient::Clip(text)))
            .collect();

        for client in clients
            .iter_mut()
            .filter(|client| !client.gone && client.session == id)
        {
            for bytes in &yanked {
                if client.link.send(bytes).is_err() {
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
        plugins.catch_up(id, steps);
        clients.retain(|client| !client.gone);

        match end {
            // The session is kept and its clients leave: with one Lua layer there is no client
            // the op belongs to more than another.
            Some(End::Detach) => clients.retain(|client| client.session != id),
            Some(_) => {
                end_session(id, plugins, sessions, clients);
                // The server is over when the last of them is: a session that quits takes
                // itself, and only itself, unless it was the only one.
                over = sessions.live.is_empty();
            }
            None => {}
        }
    }
    apply_server_ops(
        asked,
        plugins,
        sessions,
        clients,
        serving.terminals,
        serving.waker,
    );
    // Every op a key raised has been carried out, so what its client sent after it can go.
    for client in clients.iter_mut() {
        client.held = false;
    }
    // After the ops: a `session.kill` of the last one is the server being over too.
    if over || sessions.live.is_empty() {
        return Ok(true);
    }

    // The rest of the frame's time belongs to whatever the clients have to say and to the
    // panes: a key reaches its shell the moment it arrives, and the panes' output is the cue
    // to start the next frame rather than to wait the deadline out — but no sooner than a
    // `FRAME` after the last frame it started, so a pane writing a line a millisecond is drawn
    // sixty times a second rather than a thousand. The first output after a quiet spell is
    // drawn at once, which is what keeps an echoed key from waiting.
    let mut live: Vec<usize> = Vec::with_capacity(clients.len() + 1);
    let frame_end = Instant::now() + FRAME;
    let floor = cue.map(|at| at + FRAME);
    // Output has arrived and the frame it starts is waiting for the floor.
    let mut owed = false;
    loop {
        if serving.shutdown.raised().is_some() {
            return Ok(true);
        }
        // What is already in the links' buffers, before anything is waited on. The poll only
        // reports what the kernel still holds, and the door reads a connection as far as it
        // has spoken — so the key a client wrote in the same breath as its attach is in the
        // buffer already, and waiting for it is waiting for a message that has arrived.
        for i in 0..clients.len() {
            if !clients[i].gone {
                deliver(clients, i, sessions, plugins);
            }
        }
        let now = Instant::now();
        if now >= frame_end || (owed && floor.is_none_or(|floor| now >= floor)) {
            if owed {
                *cue = Some(now);
            }
            break;
        }
        live.clear();
        // Not a held client: what it has is waiting on purpose, and would wake this every time.
        live.extend((0..clients.len()).filter(|&i| !clients[i].gone && !clients[i].held));
        let deadline = match floor {
            Some(floor) if owed => floor.min(frame_end),
            _ => frame_end,
        };
        let ready = wait_over(clients, &live, serving.wakeup, deadline)?;
        match ready {
            neotmux_sys::tty::Any::Ready(slot) => match live.get(slot).copied() {
                Some(i) => {
                    if still_reading(&mut clients[i]) {
                        deliver(clients, i, sessions, plugins);
                    }
                }
                // The panes: their output is the cue to start the next frame. While that frame
                // waits for the floor the output is still read, so the children are not held
                // back to the pace of the drawing.
                None => {
                    serving.wakeup.clear();
                    for held in sessions.live.values_mut() {
                        held.session.drain()?;
                    }
                    owed = true;
                }
            },
            neotmux_sys::tty::Any::HangUp(slot) => {
                if let Some(&i) = live.get(slot) {
                    clients[i].gone = true;
                }
            }
            neotmux_sys::tty::Any::Idle => {}
        }
    }
    clients.retain(|client| !client.gone);
    Ok(false)
}

/// Everything one client has already sent, acted on. A client that sends something the
/// session cannot act on out here loses its connection and nothing else: letting it out of
/// here would end the session, and with it every other client's shells.
fn deliver(clients: &mut [Attached], i: usize, sessions: &mut Sessions, plugins: &mut Plugins) {
    // Everything this client says is about the session it is looking at.
    let which = clients[i].session;
    while !clients[i].held
        && let Some(message) = next_message(&mut clients[i])
    {
        match wire::decode_to_server(&message) {
            Ok(ToServer::Key(key)) => {
                // Offered to the plugins first and passed on only if nobody wanted it: which
                // pane an unbound key belongs to is the session's to know, and both halves
                // are here now.
                let (outcome, raised) = plugins.key(which, key);
                if outcome == KeyOutcome::Fallthrough
                    && let Some(session) = sessions.get_mut(which)
                    && session.key(key).is_err()
                {
                    clients[i].gone = true;
                }
                clients[i].held = raised;
            }
            Ok(ToServer::Resize(size)) => {
                clients[i].size = size;
                if let Some(session) = sessions.get_mut(which)
                    && session.resize(size).is_err()
                {
                    clients[i].gone = true;
                }
            }
            Ok(ToServer::Attach(_))
            | Ok(ToServer::List)
            | Ok(ToServer::Kill { .. })
            | Ok(ToServer::Command { .. }) => {
                // A second attach, a question only the door answers, or a command: none of
                // them is something the session can act on out here.
            }
            Err(err) => {
                logging::log(
                    Level::Warn,
                    "neotmux-server",
                    &format!("an attached client sent something that would not decode: {err}"),
                );
                clients[i].gone = true;
            }
        }
        // A client dropped for what it sent does not go on typing into the shared pane:
        // whatever else is in its inbox goes with it.
        if clients[i].gone {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd as _;

    use super::*;

    /// Two sessions asking for a reload in the same frame reload the config once, as the
    /// first to ask meant it — not twice, and not as whichever happened to be last in the
    /// vec.
    #[test]
    fn two_reloads_in_one_frame_reload_once() {
        let first = SessionId::from(1);
        let second = SessionId::from(2);
        let asked = vec![
            (first, "one".to_string(), ServerOp::Reload),
            (second, "two".to_string(), ServerOp::Reload),
        ];
        assert_eq!(reload_requested(&asked), Some(first));
    }

    #[test]
    fn no_reload_asked_is_none() {
        let asked = vec![(
            SessionId::from(1),
            "one".to_string(),
            ServerOp::Kill {
                name: "x".to_string(),
            },
        )];
        assert_eq!(reload_requested(&asked), None);
    }

    /// The link reads a client as a blocking socket, so one accepted off the non-blocking
    /// listener must not come out non-blocking, which is what macOS would otherwise hand over.
    #[test]
    fn a_taken_client_is_blocking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let listener = UnixListener::bind(dir.path().join("s.sock")).expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let _client = UnixStream::connect(dir.path().join("s.sock")).expect("connect");
        let stream = taken(&listener).expect("accept").expect("a client");
        let flags =
            nix::fcntl::fcntl(stream.as_raw_fd(), nix::fcntl::FcntlArg::F_GETFL).expect("F_GETFL");
        assert_eq!(flags & nix::libc::O_NONBLOCK, 0);
    }
}

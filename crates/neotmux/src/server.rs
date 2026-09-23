//! The process that holds the session.
//!
//! It has no terminal: it owns the panes, their children, the tree they are laid out in and
//! the terminal state behind each of them, and it never looks at it. It owns the Lua layer
//! too, one for the session and not one per client, so a frame is no longer a conversation:
//! the session says what happened, the plugins answer, the session applies that, and every
//! attached client is sent the picture that came of it.
//!
//! Its life is the session's: when a client goes away the session is kept until its last
//! shell exits, still reading its children, so a disconnect is not a crash. A client that
//! arrives afterwards is shown that same session — and so is one that arrives while any
//! one else is looking, the one the old code refused with a reason, and the two
//! difference being the one the other is not in. Which clients get to look at a session
//! is as far as this file goes; that a session sits waiting at all is the rest of it.

use std::collections::BTreeMap;
use std::os::fd::AsFd as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::msg::{Attach, Done, End, ToClient, ToServer};
use neotmux_proto::wire;
use neotmux_proto::{Level, SessionId, Size};

use neotmux_core::windows::PaneIds;
use neotmux_term::terminals::PaneTerminals;
use neotmux_term::wakeup::{Waker, Wakeup, wakeup};

use crate::signals::ShutdownSignal;
use neotmux::clip::Clip;
use neotmux::link::Link;
use neotmux::logging;
use neotmux::plugins::{Plugins, ServerOp};
use neotmux::reads;
use neotmux::session::{Pass, Session, Setup};

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

/// One client the frame reaches, and everything the server has to remember about it. Which
/// is very little: the picture is the session's, one Lua layer's, so every client attached
/// is shown the same one and there is nothing per client to compare it against.
struct Attached {
    link: Link,
    /// The session this client is looking at. It moves between them without its connection
    /// being touched, which is what `session.switch` is.
    session: SessionId,
    /// How big its terminal is, so a session it switches to can be refitted to it — the same
    /// `window-size latest` rule an attach follows.
    size: Size,
    /// Where the session's transcript was when this client attached: what it is told on its
    /// way out is what was raised while it was here.
    mark: usize,
    /// The client has gone, in this frame. Recorded rather than dropped, because the frame's
    /// slot is still a slot the wait is waiting over, and dropping it out from under a wait
    /// is what this flag prevents.
    gone: bool,
}

/// Every session this server holds, and the ids still to come.
///
/// One entry today — the map is the shape the rest of them go in, and everything that used to
/// say "the session" says "this one" through it.
#[derive(Default)]
struct Sessions {
    live: BTreeMap<SessionId, Held>,
    next: u32,
    /// Handed to every session this server makes. A pane's terminal state is looked up by id
    /// out of one table for the whole server, so two sessions numbering their panes apart
    /// would have them sharing a shell — and ending one would close the other's.
    panes: PaneIds,
}

/// One session and the name it answers to.
struct Held {
    name: String,
    session: Session,
}

impl Sessions {
    /// The id a new session takes. Never recycled, so an id names one session for the whole
    /// life of the server.
    fn mint(&mut self) -> SessionId {
        self.next += 1;
        SessionId::from(self.next)
    }

    fn id_of(&self, name: &str) -> Option<SessionId> {
        self.live
            .iter()
            .find(|(_, held)| held.name == name)
            .map(|(id, _)| *id)
    }

    /// Every session's name, in name order, which is the order `ls` prints them in.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.live.values().map(|held| held.name.clone()).collect();
        names.sort();
        names
    }

    fn get_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.live.get_mut(&id).map(|held| &mut held.session)
    }
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
    /// The panes' terminal state, made before the session so the Lua layer can read it.
    terminals: &'a Rc<PaneTerminals>,
}

pub fn serve(listener: &UnixListener, shutdown: &ShutdownSignal) -> anyhow::Result<()> {
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
    let lua = LuaSubsystem::build_with(layers, terminals.clone(), clip.clone())?;
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
                    steps: Vec::new(),
                    frame: None,
                    end: Some(End::Quit),
                })));
                // Held open until the client has read that and gone. A server that exited the
                // moment it wrote would have the client's own attach fail with a broken pipe,
                // and a broken pipe is not what happened here.
                let until = Instant::now() + DOORSTEP;
                while Instant::now() < until {
                    match crate::tty::wait_any(&[link.as_fd()], until) {
                        Ok(crate::tty::Any::Ready(_)) => match link.fill() {
                            Ok(true) => continue,
                            _ => break,
                        },
                        Ok(crate::tty::Any::Idle) => continue,
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

/// Waiting for the client a server with no session was started for.
///
/// A server that never got one is not a session worth keeping, and that is the whole of what
/// is waited for here. A session whose client has left is not waiting for anything: it goes
/// on running frames — the panes drained, the plugins ticked — because that is what detach
/// means now that the plugins are here.
///
/// `true` is the session going on; `false` is the server being over, and the loop does not
/// run again.
fn await_client(
    clients: &mut Vec<Attached>,
    pending: &mut Vec<Pending>,
    plugins: &mut Plugins,
    sessions: &mut Sessions,
    serving: &Serving<'_>,
    born: Instant,
) -> anyhow::Result<bool> {
    if serving.shutdown.raised().is_some() {
        return Ok(false);
    }
    if !sessions.live.is_empty() {
        return Ok(true);
    }
    if Instant::now() >= born + STARTUP {
        return Ok(false);
    }
    // The door, looked at every pass rather than only on a ready listener: a connection taken
    // in on an earlier pass is still half-way through its handshake, and a poll answers for
    // the listener, not for it. Nothing here waits.
    door(pending, clients, plugins, sessions, serving)?;
    if sessions.live.is_empty() {
        let until = (born + STARTUP).min(Instant::now() + FRAME);
        crate::tty::wait_on(std::os::fd::AsFd::as_fd(serving.listener), None, until)
            .context("wait for a client")?;
    }
    Ok(true)
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
    /// It is not a client at all: it asked a question about the sessions, and the answer is
    /// the whole of what it wanted.
    Asking(Ask),
    /// It is over: hung up, out of patience, or something that was not an attach — the one
    /// path that still says no to a connection, for the reason the old `attached` gave.
    Gone,
}

/// What a connection that is not a client wanted.
enum Ask {
    List,
    Kill { session: String },
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
            // The same law the command line enforces. A peer that did not come through the
            // command line is not exempt from it: the name is a key in a map and a word in a
            // list, and both of those are the CLI's business too.
            Ok(ToServer::Attach(attach)) => {
                match neotmux::sock::validate_session_name(&attach.session) {
                    Ok(()) => Doorstep::Attaching(attach),
                    Err(why) => {
                        logging::log(Level::Warn, "neotmux-server", &why);
                        Doorstep::Gone
                    }
                }
            }
            Ok(ToServer::List) => Doorstep::Asking(Ask::List),
            Ok(ToServer::Kill { session }) => Doorstep::Asking(Ask::Kill { session }),
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
/// `true` is somebody having been adopted.
fn door(
    pending: &mut Vec<Pending>,
    clients: &mut Vec<Attached>,
    plugins: &mut Plugins,
    sessions: &mut Sessions,
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
            Doorstep::Asking(ask) => answer(&mut one.link, ask, plugins, sessions, clients),
            Doorstep::Attaching(attach) => {
                clients.push(admit(
                    one.link,
                    &attach,
                    plugins,
                    sessions,
                    serving.terminals,
                    serving.waker,
                )?);
                adopted = true;
            }
        }
    }
    *pending = waiting;
    Ok(adopted)
}

/// What a plugin asked of the set of sessions, applied once every session has had its frame.
fn apply_server_ops(
    asked: Vec<(SessionId, String, ServerOp)>,
    plugins: &Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
    terminals: &Rc<PaneTerminals>,
    waker: &Waker,
) {
    for (from, tag, op) in asked {
        match op {
            ServerOp::Create { name } => {
                if let Err(why) = neotmux::sock::validate_session_name(&name) {
                    logging::log(
                        Level::Error,
                        &tag,
                        &format!("session.create refused: {why}"),
                    );
                    continue;
                }
                if sessions.id_of(&name).is_some() {
                    logging::log(
                        Level::Error,
                        &tag,
                        &format!("session.create('{name}') refused: it already exists"),
                    );
                    continue;
                }
                // At the size of the session that asked: nobody is looking at the new one
                // yet, and a surface has to be some size before a shell can open on it.
                let Some(size) = sessions.get_mut(from).map(|session| session.surface_size())
                else {
                    continue;
                };
                let made = Session::new(
                    &setup_for(plugins, size),
                    waker.clone(),
                    terminals.clone(),
                    sessions.panes.clone(),
                );
                match made {
                    Ok(session) => {
                        let id = sessions.mint();
                        sessions.live.insert(
                            id,
                            Held {
                                name: name.clone(),
                                session,
                            },
                        );
                        plugins.lua().name_session(id, &name);
                    }
                    Err(err) => logging::log(
                        Level::Error,
                        &tag,
                        &format!("session.create('{name}') failed: {err:#}"),
                    ),
                }
            }
            ServerOp::Switch { name } => match sessions.id_of(&name) {
                Some(to) => {
                    let mut moved = None;
                    for client in clients.iter_mut().filter(|client| client.session == from) {
                        client.session = to;
                        moved = Some(client.size);
                    }
                    // Refitted to whoever just arrived and dirty by construction, so the next
                    // frame is one they are shown — the same thing an attach does, for the
                    // same reason.
                    if let Some(size) = moved
                        && let Some(session) = sessions.get_mut(to)
                        && let Err(err) = session.adopt(&setup_for(plugins, size))
                    {
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
                if let Err(why) = neotmux::sock::validate_session_name(&to) {
                    logging::log(
                        Level::Error,
                        &tag,
                        &format!("session.rename refused: {why}"),
                    );
                    continue;
                }
                if sessions.id_of(&to).is_some() {
                    logging::log(
                        Level::Error,
                        &tag,
                        &format!("session.rename to '{to}' refused: it already exists"),
                    );
                    continue;
                }
                match sessions.id_of(&from) {
                    Some(id) => {
                        if let Some(held) = sessions.live.get_mut(&id) {
                            held.name = to.clone();
                        }
                        // The Lua layer keeps its own copy, for `session.current()` and for
                        // the list a picker draws.
                        plugins.lua().name_session(id, &to);
                    }
                    None => logging::log(
                        Level::Error,
                        &tag,
                        &format!(
                            "session.rename('{from}', '{to}') refused: no session by that name"
                        ),
                    ),
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
        }
    }
}

/// What a session is started or refitted with, out of the Lua layer and a size.
fn setup_for(plugins: &Plugins, size: Size) -> Setup {
    let lua = plugins.lua();
    Setup {
        size,
        first_window: lua.reserve_window_id(),
        reserved: lua.reserved(),
        gutter: lua.opt().pane_gutter,
        wants_pane_output: lua.has_subscriber("pane_output"),
    }
}

/// A connection that asked rather than attached, answered and let go. It is not a client, so
/// it never enters `clients` and never sees a frame.
fn answer(
    link: &mut Link,
    ask: Ask,
    plugins: &Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
) {
    match ask {
        Ask::List => {
            let names = sessions.names();
            let _ = link.send(&wire::encode_to_client(&ToClient::Sessions(&names)));
        }
        Ask::Kill { session } => {
            let found = match sessions.id_of(&session) {
                Some(id) => {
                    end_session(id, plugins, sessions, clients);
                    true
                }
                None => false,
            };
            let _ = link.send(&wire::encode_to_client(&ToClient::Killed(found)));
        }
    }
}

/// A session ends: its clients are told, its shells go with it — `Shells::drop` closes every
/// child — and the Lua layer forgets what it was holding for it.
fn end_session(
    id: SessionId,
    plugins: &Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
) {
    for client in clients.iter_mut().filter(|client| client.session == id) {
        let lines = plugins.notifications().since(client.mark);
        let _ = client
            .link
            .send(&wire::encode_to_client(&ToClient::Bye(&lines)));
        let _ = client
            .link
            .send(&wire::encode_to_client(&ToClient::Done(&Done {
                steps: Vec::new(),
                frame: None,
                end: Some(End::Quit),
            })));
        client.gone = true;
    }
    clients.retain(|client| !client.gone);
    sessions.live.remove(&id);
    plugins.lua().forget_session(id);
}

/// The client a session adopts, out of the attach it sent — and, where there is no session,
/// the session it is the first of.
fn admit(
    link: Link,
    attach: &Attach,
    plugins: &Plugins,
    sessions: &mut Sessions,
    terminals: &Rc<PaneTerminals>,
    waker: &Waker,
) -> anyhow::Result<Attached> {
    // The four the attach used to carry, asked of the Lua layer that knows them.
    let lua = plugins.lua();
    let setup = Setup {
        size: attach.size,
        first_window: lua.reserve_window_id(),
        reserved: lua.reserved(),
        gutter: lua.opt().pane_gutter,
        wants_pane_output: lua.has_subscriber("pane_output"),
    };
    let session = match sessions.id_of(&attach.session) {
        Some(id) => {
            if let Some(session) = sessions.get_mut(id) {
                session.adopt(&setup)?;
            }
            id
        }
        None => {
            let id = sessions.mint();
            sessions.live.insert(
                id,
                Held {
                    name: attach.session.clone(),
                    session: Session::new(
                        &setup,
                        waker.clone(),
                        terminals.clone(),
                        sessions.panes.clone(),
                    )?,
                },
            );
            plugins.lua().name_session(id, &attach.session);
            id
        }
    };
    Ok(Attached {
        link,
        session,
        size: attach.size,
        mark: plugins.notifications().mark(),
        gone: false,
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
) -> anyhow::Result<bool> {
    // The door is looked at once a frame rather than polled on every wait: a client that
    // arrived while the previous frame was still running is adopted here, before the tick,
    // so it is in the frame it arrived in.
    door(pending, clients, plugins, sessions, serving)?;
    clip.attached(clients.len());

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
        let Some(live) = sessions.get_mut(id) else {
            continue;
        };

        // The whole of what used to be four messages: the session says what happened, the Lua
        // layer answers, the session applies that, and the picture comes of it.
        let tick = live.tick()?;
        let answer = plugins.respond(id, tick);
        asked.extend(answer.server.into_iter().map(|(tag, op)| (id, tag, op)));
        let Pass { steps, frame, end } = live.advance(&answer.tock, &answer.views)?;
        let picture = (frame.is_some() || end.is_some()).then(|| {
            wire::encode_to_client(&ToClient::Done(&Done {
                // Nothing on the client has to be caught up any more: the steps go to a Lua
                // layer in this process, below.
                steps: Vec::new(),
                frame: frame.as_ref(),
                end,
            }))
        });
        drop(frame);

        // Taken after this session's tick, so a yank belongs to the session whose plugin
        // raised it and reaches the clients looking at that one.
        let yanked: Vec<Vec<u8>> = clip
            .take()
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
            // Before the frame that carries the end, so a client that reads its end and stops
            // reading has already been told what to say on its way out.
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
    // After the ops: a `session.kill` of the last one is the server being over too.
    if over || sessions.live.is_empty() {
        return Ok(true);
    }

    // The rest of the frame's time belongs to whatever the clients have to say and to the
    // panes: a key reaches its shell the moment it arrives, and the panes' output is the cue
    // to start the next frame rather than to wait the deadline out.
    let mut live: Vec<usize> = Vec::with_capacity(clients.len() + 1);
    let frame_end = Instant::now() + FRAME;
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
        if Instant::now() >= frame_end {
            break;
        }
        live.clear();
        live.extend((0..clients.len()).filter(|&i| !clients[i].gone));
        let ready = wait_over(clients, &live, serving.wakeup, frame_end)?;
        match ready {
            crate::tty::Any::Ready(slot) => match live.get(slot).copied() {
                Some(i) => {
                    if still_reading(&mut clients[i]) {
                        deliver(clients, i, sessions, plugins);
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
    Ok(false)
}

/// Everything one client has already sent, acted on. A client that sends something the
/// session cannot act on out here loses its connection and nothing else: letting it out of
/// here would end the session, and with it every other client's shells.
fn deliver(clients: &mut [Attached], i: usize, sessions: &mut Sessions, plugins: &mut Plugins) {
    // Everything this client says is about the session it is looking at.
    let which = clients[i].session;
    while let Some(message) = next_message(&mut clients[i]) {
        match wire::decode_to_server(&message) {
            Ok(ToServer::Key(key)) => {
                // Offered to the plugins first and passed on only if nobody wanted it: which
                // pane an unbound key belongs to is the session's to know, and both halves
                // are here now.
                if plugins.key(which, key) == KeyOutcome::Fallthrough
                    && let Some(session) = sessions.get_mut(which)
                    && session.key(key).is_err()
                {
                    clients[i].gone = true;
                }
            }
            Ok(ToServer::Resize(size)) => {
                clients[i].size = size;
                if let Some(session) = sessions.get_mut(which)
                    && session.resize(size).is_err()
                {
                    clients[i].gone = true;
                }
            }
            Ok(ToServer::Read(read)) => {
                if let Some(session) = sessions.get_mut(which) {
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
            Ok(ToServer::Tock(_))
            | Ok(ToServer::Attach(_))
            | Ok(ToServer::List)
            | Ok(ToServer::Kill { .. }) => {
                // A tock nobody asked for, a second attach, or a question only the door
                // answers: none of them is something the session can act on out here.
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

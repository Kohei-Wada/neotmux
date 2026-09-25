use crate::Attached;
use crate::Held;
use crate::Plugins;
use crate::Sessions;
use crate::server::DOORSTEP;
use crate::server::FRAME;
use crate::server::STARTUP;
use crate::server::Serving;
use crate::server::taken;
use crate::sessions::end_session;
use crate::{Session, Setup};
use anyhow::Context as _;
use neotmux_proto::Level;
use neotmux_proto::wire;
use neotmux_proto::wire::{Attach, ToClient, ToServer};
use neotmux_pty::PaneTerminals;
use neotmux_pty::child::Waker;
use neotmux_sys::Link;
use neotmux_sys::logging;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
pub(crate) const GRACE: Duration = Duration::from_millis(10);

/// Waiting for the client a server with no session was started for.
///
/// A server that never got one is not a session worth keeping, and that is the whole of what
/// is waited for here. A session whose client has left is not waiting for anything: it goes
/// on running frames — the panes drained, the plugins ticked — because that is what detach
/// means now that the plugins are here.
///
/// `true` is the session going on; `false` is the server being over, and the loop does not
/// run again.
pub(crate) fn await_client(
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
        neotmux_sys::tty::wait_on(std::os::fd::AsFd::as_fd(serving.listener), None, until)
            .context("wait for a client")?;
    }
    Ok(true)
}

/// A connection that has arrived and not yet said what it is.
///
/// It is not a client, and no frame ever waits on it: the door is looked at once a frame and
/// what is found there is read only as far as it has already spoken. A peer that connects and
/// then goes quiet sits here until its patience runs out, holding nothing but its own place —
/// the rule the old `attached` kept, now kept without the frame paying for it.
pub(crate) struct Pending {
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
pub(crate) enum Ask {
    List,
    Kill {
        session: String,
    },
    Command {
        session: Option<String>,
        name: String,
        args: Vec<String>,
    },
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
        let ready = match neotmux_sys::tty::wait_any(&[pending.link.as_fd()], until) {
            Ok(neotmux_sys::tty::Any::Ready(_)) => true,
            Ok(neotmux_sys::tty::Any::Idle) => false,
            Ok(neotmux_sys::tty::Any::HangUp(_)) => return Doorstep::Gone,
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
                match neotmux_sys::sock::validate_session_name(&attach.session) {
                    Ok(()) => Doorstep::Attaching(attach),
                    Err(why) => {
                        logging::log(Level::Warn, "neotmux-server", &why);
                        Doorstep::Gone
                    }
                }
            }
            Ok(ToServer::List) => Doorstep::Asking(Ask::List),
            Ok(ToServer::Kill { session }) => Doorstep::Asking(Ask::Kill { session }),
            Ok(ToServer::Command {
                session,
                name,
                args,
            }) => Doorstep::Asking(Ask::Command {
                session,
                name,
                args,
            }),
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
pub(crate) fn door(
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
            Doorstep::Asking(ask) => {
                answer(&mut one.link, ask, plugins, sessions, clients, serving)
            }
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
        gone: false,
    })
}

/// A connection that asked rather than attached, answered and let go. It is not a client, so
/// it never enters `clients` and never sees a frame.
pub(crate) fn answer(
    link: &mut Link,
    ask: Ask,
    plugins: &mut Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
    serving: &Serving<'_>,
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
        Ask::Command {
            session,
            name,
            args,
        } => {
            let mut ctx = crate::command::Ctx {
                sessions,
                plugins,
                terminals: serving.terminals,
                waker: serving.waker,
                clients,
            };
            let ran = crate::command::run(&name, &args, session.as_deref(), &mut ctx);
            let _ = link.send(&wire::encode_to_client(&ToClient::Ran {
                ok: ran.ok,
                lines: &ran.lines,
            }));
        }
    }
}

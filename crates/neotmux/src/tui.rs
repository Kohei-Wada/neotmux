use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use neotmux_lua::KeyOutcome;
use neotmux_proto::msg::OwnedDone;
use neotmux_proto::msg::{Attach, ToServer};
use neotmux_proto::wire;
use neotmux_proto::{OwnedToClient, Size};
use neotmux_term::state::clamp_to_min_grid;
use ratatui::DefaultTerminal;

use neotmux::decode::Decoder;
use neotmux::link::Link;
use neotmux::plugins::Plugins;
use neotmux::reads::RemotePanes;
use neotmux_proto::keys::Key;

use crate::draw;
use crate::signals::{self, ShutdownSignal};
use crate::tty::{self, Ready, Terminal};

/// How long one pass of the loop waits before looking up. The server sets the frame rate;
/// this is only how often the client notices a signal or a resize with nothing else going on.
const PATIENCE: Duration = Duration::from_millis(16);

pub fn terminal_size(terminal: &DefaultTerminal) -> anyhow::Result<Size> {
    let area = terminal.size().context("terminal size")?;
    Ok(clamp_to_min_grid(Size {
        rows: area.height,
        cols: area.width,
    }))
}

/// What one read of the terminal produced.
enum Typed {
    Keys(Vec<Key>),
    /// End of file rather than bytes: the terminal is gone.
    HangUp,
}

/// The terminal's bytes, decoded into keys.
struct Keyboard {
    tty: Terminal,
    decoder: Decoder,
    bytes: Vec<u8>,
}

impl Keyboard {
    fn open() -> anyhow::Result<Self> {
        Ok(Self {
            tty: Terminal::open().context("open the terminal for input")?,
            decoder: Decoder::default(),
            bytes: Vec::new(),
        })
    }

    fn typed(&mut self) -> anyhow::Result<Typed> {
        self.bytes.clear();
        if self.tty.read(&mut self.bytes).context("read input")? == 0 {
            return Ok(Typed::HangUp);
        }
        Ok(Typed::Keys(self.decoder.feed(&self.bytes)))
    }
}

/// What a client says when it takes a session over: everything about the session that only
/// this side knows — how big the terminal is, what the plugins reserved of it, and where
/// window ids are up to.
pub fn attach(terminal: &DefaultTerminal, plugins: &Plugins) -> anyhow::Result<Attach> {
    let lua = plugins.lua();
    Ok(Attach {
        size: terminal_size(terminal)?,
        first_window: lua.reserve_window_id(),
        reserved: lua.reserved(),
        gutter: lua.opt().pane_gutter,
        wants_pane_output: lua.has_subscriber("pane_output"),
    })
}

/// The process with the screen: the terminal, the raw mode, the key decoding, the drawing —
/// and the plugins, which M3 puts here so that one that freezes costs one client rather than
/// everybody's shells.
///
/// It never touches a pane. A key nobody bound is a message, a resize is a message, and the
/// picture that comes back is a message; what is behind them is the server's.
pub fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
    plugins: &mut Plugins,
    link: &Rc<RefCell<Link>>,
    panes: &RemotePanes,
    attach: Attach,
) -> anyhow::Result<Option<i32>> {
    let mut keyboard = Keyboard::open()?;
    let winch = signals::WindowChange::install()?;
    send(link, &ToServer::Attach(attach))?;
    // The terminal is not read until the first round is over. Until then the Lua layer has
    // been told nothing, and a key typed at the prompt before neotmux took over would reach
    // an `on_key` handler that believes the session is empty.
    let mut settled = false;

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Some(signal));
        }
        if winch.taken() {
            let size = terminal_size(terminal)?;
            send(link, &ToServer::Resize(size))?;
        }

        let deadline = Instant::now() + PATIENCE;
        let ready = {
            let peer = link.borrow();
            if settled {
                keyboard.tty.wait_beside(peer.as_fd(), deadline)
            } else {
                tty::wait_on(peer.as_fd(), None, deadline).map(|ready| match ready {
                    // The socket is the first descriptor in that wait, so both what it
                    // reports as input and what it reports as a hangup are the server's.
                    Ready::Input | Ready::HangUp => Ready::Output,
                    other => other,
                })
            }
            .context("wait for input")?
        };
        match ready {
            Ready::HangUp => return Ok(Some(signals::HANGUP)),
            Ready::Input => match keyboard.typed()? {
                Typed::HangUp => return Ok(Some(signals::HANGUP)),
                Typed::Keys(keys) => {
                    for key in keys {
                        // Offered to the plugins first and sent on only if nobody wanted it:
                        // which pane an unbound key belongs to is the session's to know.
                        if plugins.key(key) == KeyOutcome::Fallthrough {
                            send(link, &ToServer::Key(key))?;
                        }
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
                OwnedToClient::Tick(mut tick) => {
                    panes.publish(std::mem::take(&mut tick.revisions));
                    // The whole exchange, with the keyboard left alone until it is over: the
                    // ops this reply carries are applied at the far end, and a key handler
                    // that ran in between would see a session that had already moved on.
                    let tock = plugins.respond(tick);
                    send(link, &ToServer::Tock(tock))?;
                    let done = round(link)?;
                    plugins.catch_up(done.steps);
                    if let Some(frame) = &done.frame {
                        draw::show(terminal, &frame.frame())?;
                    }
                    if done.end.is_some() {
                        return Ok(None);
                    }
                    settled = true;
                }
                OwnedToClient::Refused(why) => anyhow::bail!("{why}"),
                // A finished frame outside a round, or a read nobody asked for: neither is
                // something this loop can be in the middle of.
                OwnedToClient::Done(_) | OwnedToClient::Reading(_) => {}
            }
        }
    }
}

/// The far end of the round: the tick's reply has gone, and nothing else may happen here
/// until the answer comes back.
fn round(link: &Rc<RefCell<Link>>) -> anyhow::Result<OwnedDone> {
    loop {
        let mut peer = link.borrow_mut();
        let message = match peer.recv().context("read from the server")? {
            Some(message) => message,
            None => return Err(gone(&mut peer)),
        };
        drop(peer);
        match wire::decode_to_client(&message).context("decode a server message")? {
            OwnedToClient::Done(done) => return Ok(done),
            OwnedToClient::Refused(why) => anyhow::bail!("{why}"),
            OwnedToClient::Tick(_) | OwnedToClient::Reading(_) => {}
        }
    }
}

fn send(link: &Rc<RefCell<Link>>, msg: &ToServer) -> anyhow::Result<()> {
    let mut peer = link.borrow_mut();
    if peer.send(&wire::encode_to_server(msg)).is_ok() {
        return Ok(());
    }
    Err(gone(&mut peer))
}

/// Why there is nobody to talk to. A server that turned this client away says so and then
/// closes, and the refusal is still in the socket after it has: reading it is what makes
/// "this session already has a client attached" the message rather than a broken pipe.
fn gone(peer: &mut Link) -> anyhow::Error {
    while matches!(peer.fill(), Ok(true)) {}
    while let Ok(Some(message)) = peer.take() {
        if let Ok(OwnedToClient::Refused(why)) = wire::decode_to_client(&message) {
            return anyhow::anyhow!("{why}");
        }
    }
    anyhow::anyhow!("the neotmux server went away")
}

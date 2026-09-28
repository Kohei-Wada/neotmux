use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use neotmux_proto::clamp_to_min_grid;
use neotmux_proto::wire;
use neotmux_proto::wire::{Attach, OwnedDone, ToServer};
use neotmux_proto::{OwnedToClient, Size};
use ratatui::DefaultTerminal;

use crate::decoder::Decoder;
use neotmux_proto::Key;
use neotmux_sys::Link;

use neotmux_sys::tty::{Ready, Terminal};
use neotmux_sys::{ShutdownSignal, WindowChange, signals};

/// How long one pass of the loop waits before looking up. The server sets the frame rate;
/// this is only how often the client notices a signal or a resize with nothing else going on.
const PATIENCE: Duration = Duration::from_millis(16);

/// The terminal's size, asked of the terminal itself rather than of a renderer: it is wanted
/// before there is one, because the attach happens before the alternate screen.
pub(crate) fn screen_size() -> anyhow::Result<Size> {
    let (cols, rows) = ratatui::crossterm::terminal::size().context("terminal size")?;
    Ok(clamp_to_min_grid(Size { rows, cols }))
}

pub(crate) fn terminal_size(terminal: &DefaultTerminal) -> anyhow::Result<Size> {
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

/// Why the client stopped.
pub(crate) struct Left {
    pub signal: Option<i32>,
}

/// What the session said when this client attached.
pub(crate) enum Opened {
    /// It is over already — a plugin called `neotmux.quit()` while the session was loading.
    /// Nothing was drawn, and nothing should be.
    Over(Vec<String>),
    /// There is a session, and this is the first frame of it.
    Running(Box<OwnedDone>),
}

/// The attach, and the session's first word about it.
///
/// Asked for before the alternate screen is entered, because a session that was over before
/// it began must not leave a terminal switched to a screen nothing was ever drawn on. The
/// first frame is always owed: a session that has just started, and one that has just been
/// adopted, are both dirty by construction.
pub(crate) fn open(
    link: &Rc<RefCell<Link>>,
    size: Size,
    session: Option<String>,
) -> anyhow::Result<Opened> {
    send(link, &ToServer::Attach(Attach { size, session }))?;
    let mut transcript = Vec::new();
    loop {
        let message = {
            let mut peer = link.borrow_mut();
            match peer.recv().context("read from the server")? {
                Some(message) => message,
                None => return Err(gone(&mut peer)),
            }
        };
        match wire::decode_to_client(&message).context("decode a server message")? {
            // Over before it had a screen only when there is nothing to show: a first frame
            // that also ends the session is a shell that printed and exited before the session
            // drew once, and what it printed is in that frame.
            OwnedToClient::Done(done) if done.end.is_some() && done.frame.is_none() => {
                return Ok(Opened::Over(transcript));
            }
            OwnedToClient::Done(done) => return Ok(Opened::Running(Box::new(done))),
            OwnedToClient::Bye(lines) => transcript = lines,
            OwnedToClient::Clip(text) => yank(&text),
            OwnedToClient::Refused(why) => anyhow::bail!("{why}"),
            OwnedToClient::Sessions(_) | OwnedToClient::Killed(_) | OwnedToClient::Ran { .. } => {}
        }
    }
}

/// The process with the screen: the terminal, the raw mode, the key decoding, the drawing.
///
/// It holds no plugin and makes no decision. A key it reads is sent, not judged; the picture
/// that comes back is painted, not composed. Both of those are the session's, and the session
/// is in the other process.
pub(crate) fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
    link: &Rc<RefCell<Link>>,
    first: OwnedDone,
) -> anyhow::Result<Left> {
    let mut keyboard = Keyboard::open()?;
    let winch = WindowChange::install()?;
    if let Some(frame) = &first.frame {
        crate::draw::show(terminal, &frame.frame())?;
    }
    if first.end.is_some() {
        return Ok(Left { signal: None });
    }

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Left {
                signal: Some(signal),
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
                });
            }
            Ready::Input => match keyboard.typed()? {
                Typed::HangUp => {
                    return Ok(Left {
                        signal: Some(signals::HANGUP),
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
                        crate::draw::show(terminal, &frame.frame())?;
                    }
                    if done.end.is_some() {
                        return Ok(Left { signal: None });
                    }
                }
                OwnedToClient::Clip(text) => yank(&text),
                // Only ever sent at the door, to a client whose session was over before it
                // had a screen. An attached client is told nothing on its way out: every
                // notification it could be told is in the log file already.
                OwnedToClient::Bye(_) => {}
                OwnedToClient::Refused(why) => anyhow::bail!("{why}"),
                // The three answers only ever given at the door. Nothing sends any of these to
                // a client that has attached.
                OwnedToClient::Sessions(_)
                | OwnedToClient::Killed(_)
                | OwnedToClient::Ran { .. } => {}
            }
        }
    }
}

/// The one thing on the screen the session cannot do for itself: OSC 52, written to this
/// client's terminal and flushed. Flushing is not optional — the sequence carries no newline,
/// so a line-buffered terminal would hold it until something else came along.
fn yank(text: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = out
        .write_all(&crate::osc52::set(text))
        .and_then(|()| out.flush());
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

#[cfg(test)]
mod tests {
    use super::*;
    use neotmux_proto::wire::End;
    use std::os::unix::net::UnixStream;

    /// A client link and the server's end of it, both past the hello.
    fn pair() -> (Rc<RefCell<Link>>, Link) {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let server = std::thread::spawn(move || Link::open(a).expect("open"));
        let client = Link::open(b).expect("open");
        (Rc::new(RefCell::new(client)), server.join().expect("join"))
    }

    /// The server's side, on its own thread: it reads the attach `open` sends before it answers.
    fn answer(mut server: Link, frame: bool, end: Option<End>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || answer_now(&mut server, frame, end))
    }

    fn answer_now(server: &mut Link, frame: bool, end: Option<End>) {
        server.recv().expect("the attach").expect("an attach");
        let rects = neotmux_proto::PaneRects::new();
        let drawn = neotmux_proto::Frame {
            size: Size { rows: 2, cols: 2 },
            panes: Vec::new(),
            rects: &rects,
            cursor: None,
            overlay: &[],
        };
        let done = wire::Done {
            frame: frame.then_some(&drawn),
            end,
        };
        server
            .send(&wire::encode_to_client(&wire::ToClient::Done(&done)))
            .expect("send");
    }

    /// A shell that printed and exited before the session's first frame ends the session in
    /// that frame, and the frame is its last output: the client draws it, and does not take it
    /// for a session that was over before it had a screen.
    #[test]
    fn a_first_frame_that_is_also_the_last_is_drawn() {
        let (client, server) = pair();
        let answered = answer(server, true, Some(End::Eof));
        let opened = open(&client, Size { rows: 2, cols: 2 }, None).expect("open");
        answered.join().expect("the server's side");
        match opened {
            Opened::Running(done) => {
                assert!(done.frame.is_some());
                assert_eq!(done.end, Some(End::Eof));
            }
            Opened::Over(_) => panic!("the last frame was thrown away as a session never begun"),
        }
    }

    /// And a session a plugin ended while it loaded, which sends no frame, is still over before
    /// the screen is touched.
    #[test]
    fn an_end_with_no_frame_is_a_session_over_before_it_began() {
        let (client, server) = pair();
        let answered = answer(server, false, Some(End::Quit));
        let opened = open(&client, Size { rows: 2, cols: 2 }, None).expect("open");
        answered.join().expect("the server's side");
        assert!(matches!(opened, Opened::Over(_)));
    }
}

use std::time::{Duration, Instant};

use anyhow::Context as _;
use neotmux_lua::LuaSubsystem;
use neotmux_proto::Size;
use neotmux_term::state::clamp_to_min_grid;
use neotmux_term::wakeup::{Wakeup, wakeup};
use ratatui::DefaultTerminal;

use neotmux::decode::Decoder;
use neotmux::keys::Key;
use neotmux::notify::Notifications;
use neotmux::session::{End, Session};

use crate::draw;
use crate::signals::{self, ShutdownSignal};
use crate::tty::{Ready, Terminal};

/// How long one pass of the frame loop lasts: the tick's cadence, and the closest together
/// two draws can be. A frame no longer spends its remainder asleep — the loop pumps input
/// until the deadline instead — so the rate is still a property of the loop rather than of
/// whatever the input wait happens to do.
const FRAME: Duration = Duration::from_millis(16);

fn terminal_size(terminal: &DefaultTerminal) -> anyhow::Result<Size> {
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

/// The terminal's bytes, decoded into keys. A session is handed these and never reaches for
/// the process's terminal itself, which is what lets one be driven from somewhere else later.
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

    fn wait(&self, panes: Option<&Wakeup>, deadline: Instant) -> anyhow::Result<Ready> {
        self.tty.wait(panes, deadline).context("wait for input")
    }

    fn typed(&mut self) -> anyhow::Result<Typed> {
        self.bytes.clear();
        if self.tty.read(&mut self.bytes).context("read input")? == 0 {
            return Ok(Typed::HangUp);
        }
        Ok(Typed::Keys(self.decoder.feed(&self.bytes)))
    }
}

/// Pumps input until the frame's time is up, or — when it was given the panes' wakeup —
/// until a pane produces output, which is the frame's cue to start drawing. Returns the
/// signal to exit on if one was raised while waiting, or if the terminal itself went away.
fn pump_input(
    session: &mut Session,
    lua: &LuaSubsystem,
    shutdown: &ShutdownSignal,
    keyboard: &mut Keyboard,
    panes: Option<&Wakeup>,
    frame_end: Instant,
) -> anyhow::Result<Option<i32>> {
    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Some(signal));
        }
        // Past `frame_end` the wait returns at once, which is what still lets a frame that
        // overran collect the input waiting on it.
        let ready = keyboard.wait(panes, frame_end)?;
        if ready == Ready::HangUp {
            return Ok(Some(signals::HANGUP));
        }
        if ready == Ready::Input {
            match keyboard.typed()? {
                Typed::HangUp => return Ok(Some(signals::HANGUP)),
                Typed::Keys(keys) => {
                    for key in keys {
                        session.key(lua, key)?;
                    }
                }
            }
        }
        // Output is no exception: a key that arrived with it belongs to the frame that is
        // about to draw, not to the one after.
        if ready == Ready::Output || Instant::now() >= frame_end {
            return Ok(None);
        }
    }
}

pub fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
    lua: &LuaSubsystem,
    notifications: &mut Notifications,
) -> anyhow::Result<Option<i32>> {
    let mut keyboard = Keyboard::open()?;
    let winch = signals::WindowChange::install()?;
    let (wakeup, waker) = wakeup().context("open the panes' wakeup")?;
    let mut session = Session::new(terminal_size(terminal)?, lua, waker)?;

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Some(signal));
        }

        let frame_end = Instant::now() + FRAME;

        // The frame's own work waits for a reason to do it: a pane's output, or the deadline
        // the tick is owed. Keys are handled the moment they arrive either way.
        if let Some(signal) = pump_input(
            &mut session,
            lua,
            shutdown,
            &mut keyboard,
            Some(&wakeup),
            frame_end,
        )? {
            return Ok(Some(signal));
        }

        if winch.taken() {
            session.resize(terminal_size(terminal)?, lua)?;
        }

        let step = session.advance(lua, notifications)?;
        if step.end == Some(End::Quit) {
            return Ok(None);
        }
        if let Some(frame) = &step.frame {
            draw::show(terminal, frame)?;
        }
        let eof = step.end == Some(End::Eof);
        drop(step);

        if eof {
            break;
        }

        // The rest of the frame is the loop's rate, and it is spent on input rather than
        // asleep: keys still reach their pane at once, while a pane's output waits for the
        // next frame rather than drawing this one a second time.
        if let Some(signal) =
            pump_input(&mut session, lua, shutdown, &mut keyboard, None, frame_end)?
        {
            return Ok(Some(signal));
        }
    }

    Ok(None)
}

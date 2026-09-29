//! Attaching: the terminal, then the session, then the loop.
//!
//! The order is the whole of this file. Raw mode is taken before anything slow, the attach
//! is answered before the alternate screen is entered, and the terminal is given back
//! however the loop leaves.

use neotmux_proto::Level;
use neotmux_sys::logging;

use crate::start;

/// Raw mode, taken before anything slow and given back however [`attach`] leaves — a
/// panic included, because unwinding runs this drop.
///
/// Until it is taken the line discipline is the one reading the keyboard, and it does not
/// hand over what it understands as editing: `0x17` (`C-w`) is WERASE and is eaten rather
/// than delivered, `0x03` raises SIGINT, and `\r` arrives as `\n`. Starting a server and
/// loading the Lua layer both take long enough on a loaded machine for a key typed at the
/// prompt to land inside that window, so the terminal is claimed first and everything else
/// is done under it.
struct RawMode;

impl RawMode {
    fn enter() -> Self {
        if let Err(err) = ratatui::crossterm::terminal::enable_raw_mode() {
            logging::log(
                Level::Warn,
                "neotmux",
                &format!("could not put the terminal in raw mode: {err}"),
            );
        }
        Self
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
    }
}

/// The terminal, the session, and the loop between them, in that order.
pub fn attach(session: Option<String>) -> anyhow::Result<()> {
    // First, and before the socket: until the terminal is in raw mode the line discipline is
    // answering the keyboard, and starting a server is not fast.
    let _raw = RawMode::enter();
    let name = session.clone();
    let link = start::begin()?.finish()?;

    // The attach, and the session's answer, before the screen: a session that a plugin ended
    // while it was loading must not flash the alternate screen on its way past.
    let first = match super::open(&link, super::screen_size()?, name)? {
        super::Opened::Over(transcript) => {
            for line in &transcript {
                eprintln!("neotmux: {line}");
            }
            return Ok(());
        }
        super::Opened::Running(done) => *done,
    };

    let shutdown = neotmux_sys::ShutdownSignal::install()?;
    let mut terminal = ratatui::init();
    let result = super::run(&mut terminal, &shutdown, &link, first);
    // Not `ratatui::restore`: it reports a failure with `eprintln!`, which panics when the
    // terminal it is trying to restore is the one that has gone away.
    if let Err(err) = ratatui::try_restore() {
        logging::log(
            Level::Warn,
            "neotmux",
            &format!("could not restore the terminal: {err}"),
        );
    }
    let left = result?;
    match left.signal {
        Some(signal) => neotmux_sys::signals::die_of(signal),
        None => Ok(()),
    }
}

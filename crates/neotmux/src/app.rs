use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::event::{self, Event};
use neotmux_core::proto::{GridSnapshot, Size};
use neotmux_term::pty::{PtyEvent, PtyHandle};
use neotmux_term::state::TermState;
use ratatui::DefaultTerminal;

use crate::input::encode_key;
use crate::render::draw_grid;
use crate::signals::ShutdownSignal;

const FRAME: Duration = Duration::from_millis(16);
const SCROLLBACK: usize = 10_000;
const MAX_CHUNKS_PER_FRAME: usize = 32;

fn clamp_to_at_least_one(size: Size) -> Size {
    Size {
        rows: size.rows.max(1),
        cols: size.cols.max(1),
    }
}

fn terminal_size(terminal: &DefaultTerminal) -> anyhow::Result<Size> {
    let area = terminal.size().context("terminal size")?;
    Ok(clamp_to_at_least_one(Size {
        rows: area.height,
        cols: area.width,
    }))
}

pub fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
) -> anyhow::Result<Option<i32>> {
    let mut size = terminal_size(terminal)?;
    let mut state = TermState::new(size, SCROLLBACK);
    let mut pty = PtyHandle::spawn(PtyHandle::default_shell_command(), size)?;
    let mut grid = GridSnapshot::default();

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Some(signal));
        }

        let frame_end = Instant::now() + FRAME;

        let mut eof = false;
        for _ in 0..MAX_CHUNKS_PER_FRAME {
            match pty.output().try_recv() {
                Ok(PtyEvent::Output(bytes)) => state.feed(&bytes),
                Ok(PtyEvent::Eof) => {
                    eof = true;
                    break;
                }
                Ok(PtyEvent::Error(err)) => return Err(err).context("read from pty"),
                Err(_) => break,
            }
        }

        if !eof {
            loop {
                if let Some(signal) = shutdown.raised() {
                    return Ok(Some(signal));
                }
                let remaining = frame_end.saturating_duration_since(Instant::now());
                if !event::poll(remaining).context("poll input")? {
                    break;
                }
                match event::read().context("read input")? {
                    Event::Key(key) => {
                        if let Some(bytes) = encode_key(key) {
                            pty.write(&bytes).context("write to pty")?;
                        }
                    }
                    Event::Resize(cols, rows) => {
                        size = clamp_to_at_least_one(Size { rows, cols });
                        state.resize(size);
                        pty.resize(size)?;
                    }
                    _ => {}
                }
                if Instant::now() >= frame_end {
                    break;
                }
            }
        }

        state.snapshot_into(&mut grid);
        terminal
            .draw(|frame| draw_grid(frame, frame.area(), &grid))
            .context("draw")?;

        if eof {
            break;
        }
    }

    pty.wait().context("wait for child")?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use portable_pty::CommandBuilder;

    use super::*;

    fn quick_exit_command() -> CommandBuilder {
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "true"]);
        cmd
    }

    #[test]
    fn a_zero_size_is_clamped_the_same_for_state_and_pty() {
        let raw = Size { rows: 0, cols: 0 };
        let clamped = clamp_to_at_least_one(raw);
        assert_eq!(clamped, Size { rows: 1, cols: 1 });

        let state = TermState::new(clamped, 0);
        let mut pty = PtyHandle::spawn(quick_exit_command(), clamped).expect("spawn pty");

        assert_eq!(state.size(), clamped);
        assert_eq!(pty.size().expect("pty size"), clamped);

        pty.wait().ok();
    }

    #[test]
    fn resizing_to_zero_is_clamped_the_same_for_state_and_pty() {
        let initial = Size { rows: 24, cols: 80 };
        let mut state = TermState::new(initial, 0);
        let pty = PtyHandle::spawn(quick_exit_command(), initial).expect("spawn pty");

        let raw = Size { rows: 0, cols: 0 };
        let clamped = clamp_to_at_least_one(raw);
        state.resize(clamped);
        pty.resize(clamped).expect("resize pty");

        assert_eq!(state.size(), clamped);
        assert_eq!(pty.size().expect("pty size"), clamped);
    }
}

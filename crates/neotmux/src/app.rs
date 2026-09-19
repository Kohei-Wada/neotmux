use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::event::{self, Event};
use neotmux_core::proto::{GridSnapshot, Size};
use neotmux_term::pty::{PtyEvent, PtyHandle};
use neotmux_term::state::TermState;
use ratatui::DefaultTerminal;

use crate::input::encode_key;
use crate::render::draw_grid;

const FRAME: Duration = Duration::from_millis(16);
const SCROLLBACK: usize = 10_000;

fn terminal_size(terminal: &DefaultTerminal) -> anyhow::Result<Size> {
    let area = terminal.size().context("terminal size")?;
    Ok(Size {
        rows: area.height,
        cols: area.width,
    })
}

pub fn run(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    let mut size = terminal_size(terminal)?;
    let mut state = TermState::new(size, SCROLLBACK);
    let mut pty = PtyHandle::spawn(PtyHandle::default_shell_command(), size)?;
    let mut grid = GridSnapshot::default();

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        let frame_end = Instant::now() + FRAME;

        let mut eof = false;
        while let Ok(ev) = pty.output().try_recv() {
            match ev {
                PtyEvent::Output(bytes) => state.feed(&bytes),
                PtyEvent::Eof => eof = true,
            }
        }

        if !eof {
            while Instant::now() < frame_end {
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
                        size = Size { rows, cols };
                        state.resize(size);
                        pty.resize(size)?;
                    }
                    _ => {}
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
    Ok(())
}

use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::event::{self, Event};
use neotmux_core::proto::{DrawCmd, GridSnapshot, Size, StatusFragment};
use neotmux_lua::ops::{Level, Op};
use neotmux_lua::{Event as LuaEvent, KeyOutcome, LuaSubsystem};
use neotmux_term::pty::{PtyEvent, PtyHandle};
use neotmux_term::state::{TermState, clamp_to_min_grid};
use ratatui::DefaultTerminal;

use crate::input::{encode_key, key_spec};
use crate::logging;
use crate::notify::Notifications;
use crate::overlay;
use crate::panes::{self, PaneLayout};
use crate::render::{draw_grid, layout};
use crate::signals::ShutdownSignal;

const FRAME: Duration = Duration::from_millis(16);
const SCROLLBACK: usize = 10_000;
const MAX_CHUNKS_PER_FRAME: usize = 32;

fn terminal_size(terminal: &DefaultTerminal) -> anyhow::Result<Size> {
    let area = terminal.size().context("terminal size")?;
    Ok(clamp_to_min_grid(Size {
        rows: area.height,
        cols: area.width,
    }))
}

/// The state one frame carries into the next. Every step of a frame is a method on it, so
/// the loop in `run` reads as the order of the steps and nothing else.
struct Session {
    /// The whole terminal, status row included.
    size: Size,
    layout: PaneLayout,
    has_status: bool,
    state: TermState,
    pty: PtyHandle,
    grid: GridSnapshot,
    dirty: bool,
    last_frame_output: (Vec<DrawCmd>, Vec<StatusFragment>),
}

/// What draining the PTY found: `Eof` ends the session after one last frame.
#[derive(PartialEq, Eq)]
enum Drained {
    More,
    Eof,
}

impl Session {
    fn new(size: Size) -> anyhow::Result<Self> {
        let has_status = false;
        let layout = PaneLayout::compute(size, has_status);
        let state = TermState::new(layout.content, SCROLLBACK);
        let pty = PtyHandle::spawn(PtyHandle::default_shell_command(), layout.content)?;
        Ok(Self {
            size,
            layout,
            has_status,
            state,
            pty,
            grid: GridSnapshot::default(),
            dirty: true,
            last_frame_output: (Vec::new(), Vec::new()),
        })
    }

    /// A geometry change, whether from a real terminal resize or the status row
    /// appearing/disappearing: recompute the layout, resize `TermState` and the PTY to
    /// match, and tell the Lua layer about it (§4.8).
    fn apply_geometry(&mut self, lua: &LuaSubsystem) -> anyhow::Result<()> {
        self.layout = PaneLayout::compute(self.size, self.has_status);
        self.state.resize(self.layout.content);
        self.pty.resize(self.layout.content)?;
        lua.dispatch_event(&LuaEvent::Resized {
            size: self.layout.content,
        });
        self.dirty = true;
        Ok(())
    }

    /// At most `MAX_CHUNKS_PER_FRAME` chunks, so a flooding child cannot starve input.
    fn drain_pty(&mut self, lua: &LuaSubsystem) -> anyhow::Result<Drained> {
        for _ in 0..MAX_CHUNKS_PER_FRAME {
            match self.pty.output().try_recv() {
                Ok(PtyEvent::Output(bytes)) => {
                    self.state.feed(&bytes);
                    lua.dispatch_event(&LuaEvent::PaneOutput {
                        pane: self.layout.output_pane,
                        bytes,
                    });
                    self.dirty = true;
                }
                Ok(PtyEvent::Eof) => return Ok(Drained::Eof),
                Ok(PtyEvent::Error(err)) => return Err(err).context("read from pty"),
                Err(_) => break,
            }
        }
        Ok(Drained::More)
    }

    /// Reads input until the frame's time is up. Returns the signal to exit on if one was
    /// raised while waiting.
    fn pump_input(
        &mut self,
        lua: &LuaSubsystem,
        shutdown: &ShutdownSignal,
        frame_end: Instant,
    ) -> anyhow::Result<Option<i32>> {
        loop {
            if let Some(signal) = shutdown.raised() {
                return Ok(Some(signal));
            }
            let remaining = frame_end.saturating_duration_since(Instant::now());
            if !event::poll(remaining).context("poll input")? {
                return Ok(None);
            }
            match event::read().context("read input")? {
                Event::Key(key) => {
                    let outcome = match key_spec(&key) {
                        Some(spec) => lua.dispatch_key(&spec),
                        None => KeyOutcome::Fallthrough,
                    };
                    if outcome == KeyOutcome::Fallthrough
                        && let Some(bytes) = encode_key(key)
                    {
                        self.pty.write(&bytes).context("write to pty")?;
                    }
                }
                Event::Resize(cols, rows) => {
                    self.size = clamp_to_min_grid(Size { rows, cols });
                    self.apply_geometry(lua)?;
                }
                _ => {}
            }
            if Instant::now() >= frame_end {
                return Ok(None);
            }
        }
    }

    /// Every op a source raised this frame, applied in the order they were pushed (§4.6).
    /// Returns whether one of them asked to quit.
    fn drain_ops(&mut self, lua: &LuaSubsystem, notifications: &mut Notifications) -> bool {
        let mut quit = false;
        for (id, op) in lua.drain_ops() {
            let tag = lua.sources().name(id).unwrap_or_else(|| id.to_string());
            match op {
                Op::Quit => quit = true,
                Op::Log { level, text } => logging::log(level, &tag, &text),
                Op::Notify { level, text } => {
                    notifications.push(level, &tag, &text);
                    logging::log(level, &tag, &text);
                }
                Op::PaneSplit { dir } => logging::log(Level::Info, &tag, &panes::split_note(dir)),
                Op::PaneFocus { pane } => logging::log(Level::Info, &tag, &panes::focus_note(pane)),
            }
        }
        quit
    }

    /// Whether the status row just appeared or disappeared, which changes the pane's size.
    fn status_row_changed(&mut self, status: &[StatusFragment]) -> bool {
        let now_has_status = !status.is_empty();
        if now_has_status == self.has_status {
            return false;
        }
        self.has_status = now_has_status;
        true
    }

    fn render(
        &mut self,
        terminal: &mut DefaultTerminal,
        notifications: &mut Notifications,
        cmds: &[DrawCmd],
        status: &[StatusFragment],
    ) -> anyhow::Result<()> {
        self.state.snapshot_into(&mut self.grid);
        notifications.expire(Instant::now());

        let grid = &self.grid;
        let rects = &self.layout.rects;
        let has_status = self.has_status;
        terminal
            .draw(|frame| {
                let area = frame.area();
                let (pane_area, status_area) = layout(area, has_status);
                draw_grid(frame, pane_area, grid);
                overlay::apply(frame, pane_area, cmds, rects);
                if has_status {
                    overlay::draw_status(frame, status_area, status);
                }
                notifications.render(frame, area);
            })
            .context("draw")?;
        self.dirty = false;
        Ok(())
    }
}

pub fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
    lua: &LuaSubsystem,
    notifications: &mut Notifications,
) -> anyhow::Result<Option<i32>> {
    let mut session = Session::new(terminal_size(terminal)?)?;

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Some(signal));
        }

        let frame_end = Instant::now() + FRAME;
        let drained = session.drain_pty(lua)?;

        if drained == Drained::More
            && let Some(signal) = session.pump_input(lua, shutdown, frame_end)?
        {
            return Ok(Some(signal));
        }

        lua.begin_frame(
            session.layout.epoch(),
            session.layout.content,
            session.layout.panes(),
            session.layout.focus(),
        );
        lua.dispatch_tick();

        if session.drain_ops(lua, notifications) {
            return Ok(None);
        }

        let frame_output = lua.frame_output();
        if session.status_row_changed(&frame_output.1) {
            session.apply_geometry(lua)?;
        }

        if frame_output != session.last_frame_output {
            session.dirty = true;
        }
        // A live notification needs a later frame just to notice it expired
        // and stop being drawn, even if nothing else changed.
        if !notifications.is_empty() {
            session.dirty = true;
        }

        let eof = drained == Drained::Eof;
        if session.dirty || eof {
            let (cmds, status) = &frame_output;
            session.render(terminal, notifications, cmds, status)?;
        }
        session.last_frame_output = frame_output;

        if eof {
            break;
        }
    }

    session.pty.wait().context("wait for child")?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use neotmux_term::state::MIN_GRID;
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
        let clamped = clamp_to_min_grid(raw);
        assert_eq!(clamped, MIN_GRID);

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
        let clamped = clamp_to_min_grid(raw);
        state.resize(clamped);
        pty.resize(clamped).expect("resize pty");

        assert_eq!(state.size(), clamped);
        assert_eq!(pty.size().expect("pty size"), clamped);
    }
}

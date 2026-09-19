use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::event::{self, Event};
use neotmux_core::proto::{
    DrawCmd, Epoch, GridSnapshot, PaneId, Rect as PRect, Size, StatusFragment,
};
use neotmux_lua::ops::{Level, Op};
use neotmux_lua::{Event as LuaEvent, KeyOutcome, LuaSubsystem};
use neotmux_term::pty::{PtyEvent, PtyHandle};
use neotmux_term::state::TermState;
use ratatui::DefaultTerminal;

use crate::input::{encode_key, key_spec};
use crate::logging;
use crate::notify::Notifications;
use crate::overlay::{self, PaneRects};
use crate::panes;
use crate::render::{draw_grid, layout};
use crate::signals::ShutdownSignal;

const FRAME: Duration = Duration::from_millis(16);
const SCROLLBACK: usize = 10_000;
const MAX_CHUNKS_PER_FRAME: usize = 32;

/// The single PTY-backed pane this milestone knows about.
fn pane_id() -> PaneId {
    PaneId::from(1)
}

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

/// The pane's own size: the full terminal, minus the status row once a
/// status provider exists. A one-row terminal can't spare that row.
fn content_size(full: Size, has_status: bool) -> Size {
    if has_status && full.rows > 1 {
        Size {
            rows: full.rows - 1,
            cols: full.cols,
        }
    } else {
        full
    }
}

fn pane_rects(content: Size) -> PaneRects {
    let mut map = PaneRects::new();
    map.insert(pane_id(), PRect::new(0, 0, content.cols, content.rows));
    map
}

/// A geometry change, whether from a real terminal resize or the status row
/// appearing/disappearing: recompute the pane's size and rects, resize
/// `TermState` and the PTY to match, and tell the Lua layer about it.
fn apply_geometry(
    lua: &LuaSubsystem,
    state: &mut TermState,
    pty: &mut PtyHandle,
    size: Size,
    has_status: bool,
) -> anyhow::Result<(Size, PaneRects)> {
    let content = content_size(size, has_status);
    let panes = pane_rects(content);
    state.resize(content);
    pty.resize(content)?;
    lua.dispatch_event(&LuaEvent::Resized { size: content });
    Ok((content, panes))
}

pub fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
    lua: &LuaSubsystem,
) -> anyhow::Result<Option<i32>> {
    let mut size = terminal_size(terminal)?;
    let mut has_status = false;
    let mut content = content_size(size, has_status);
    let mut panes = pane_rects(content);
    let mut state = TermState::new(content, SCROLLBACK);
    let mut pty = PtyHandle::spawn(PtyHandle::default_shell_command(), content)?;
    let mut grid = GridSnapshot::default();
    let mut dirty = true;
    let mut notifications = Notifications::new();
    let mut last_frame_output: (Vec<DrawCmd>, Vec<StatusFragment>) = (Vec::new(), Vec::new());

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
                Ok(PtyEvent::Output(bytes)) => {
                    state.feed(&bytes);
                    lua.dispatch_event(&LuaEvent::PaneOutput {
                        pane: pane_id(),
                        bytes,
                    });
                    dirty = true;
                }
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
                        let outcome = match key_spec(&key) {
                            Some(spec) => lua.dispatch_key(&spec),
                            None => KeyOutcome::Fallthrough,
                        };
                        if outcome == KeyOutcome::Fallthrough
                            && let Some(bytes) = encode_key(key)
                        {
                            pty.write(&bytes).context("write to pty")?;
                        }
                    }
                    Event::Resize(cols, rows) => {
                        size = clamp_to_at_least_one(Size { rows, cols });
                        (content, panes) =
                            apply_geometry(lua, &mut state, &mut pty, size, has_status)?;
                        dirty = true;
                    }
                    _ => {}
                }
                if Instant::now() >= frame_end {
                    break;
                }
            }
        }

        let epoch = Epoch::new(content, vec![pane_id()], Some(pane_id()));
        lua.begin_frame(
            epoch,
            content,
            vec![(pane_id(), *panes.get(&pane_id()).unwrap())],
            Some(pane_id()),
        );
        lua.dispatch_tick();

        let mut quit = false;
        for (id, op) in lua.drain_ops() {
            match op {
                Op::Quit => quit = true,
                Op::Log { level, text } => {
                    let tag = lua.sources().name(id).unwrap_or_else(|| id.to_string());
                    logging::log(level, &tag, &text);
                }
                Op::Notify { level, text } => {
                    let tag = lua.sources().name(id).unwrap_or_else(|| id.to_string());
                    notifications.push(level, &tag, &text);
                    logging::log(level, &tag, &text);
                }
                Op::PaneSplit { dir } => {
                    let tag = lua.sources().name(id).unwrap_or_else(|| id.to_string());
                    logging::log(Level::Info, &tag, &panes::split_note(dir));
                }
                Op::PaneFocus { pane } => {
                    let tag = lua.sources().name(id).unwrap_or_else(|| id.to_string());
                    logging::log(Level::Info, &tag, &panes::focus_note(pane));
                }
            }
        }
        if quit {
            return Ok(None);
        }

        let frame_output = lua.frame_output();
        let now_has_status = !frame_output.1.is_empty();
        if now_has_status != has_status {
            has_status = now_has_status;
            (content, panes) = apply_geometry(lua, &mut state, &mut pty, size, has_status)?;
            dirty = true;
        }

        if frame_output != last_frame_output {
            dirty = true;
        }
        // A live notification needs a later frame just to notice it expired
        // and stop being drawn, even if nothing else changed.
        if !notifications.is_empty() {
            dirty = true;
        }
        let (cmds, status) = &frame_output;

        if dirty || eof {
            state.snapshot_into(&mut grid);
            notifications.expire(Instant::now());
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    let (pane_area, status_area) = layout(area, has_status);
                    draw_grid(frame, pane_area, &grid);
                    overlay::apply(frame, pane_area, cmds, &panes);
                    if has_status {
                        overlay::draw_status(frame, status_area, status);
                    }
                    notifications.render(frame, area);
                })
                .context("draw")?;
            dirty = false;
        }
        last_frame_output = frame_output;

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

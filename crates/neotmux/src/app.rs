use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::event::{self, Event};
use neotmux_core::proto::{DrawCmd, PaneId, Size, StatusFragment};
use neotmux_lua::ops::{Level, Op};
use neotmux_lua::{Event as LuaEvent, KeyOutcome, LuaSubsystem};
use neotmux_term::pty::PtyHandle;
use neotmux_term::state::clamp_to_min_grid;
use ratatui::DefaultTerminal;
use ratatui::layout::Rect;

use crate::input::{encode_key, key_spec};
use crate::logging;
use crate::notify::Notifications;
use crate::overlay;
use crate::panes::PaneLayout;
use crate::render::{cursor_position, draw_grid, layout};
use crate::shells::Shells;
use crate::signals::ShutdownSignal;

const FRAME: Duration = Duration::from_millis(16);
const MAX_CHUNKS_PER_FRAME: usize = 32;
const FIRST_PANE: u32 = 1;

/// How many `pane.split` ops one frame will act on. A split is host work a plugin buys from
/// outside its measured tick — an op pushed in the tick, a shell fork/exec'd after it — so
/// M1b's promise only holds if the number of them a frame can be made to do is bounded.
///
/// Derived rather than picked: the frame is 16 ms, the default tick budget claims 8 ms of
/// it, and draining the PTYs and rendering need the rest, so applying ops gets about half
/// of what is left, 4 ms. One split measured 1.3 ms (release, 24x40 pane: openpty, fork,
/// exec, terminal state), and 4 / 1.3 is 3.
const MAX_SPLITS_PER_FRAME: usize = 3;

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
    shells: Shells,
    /// Pane ids are never reused: a stale `pane.focus(id)` must not land on a new pane.
    next_pane: u32,
    dirty: bool,
    last_frame_output: (Vec<DrawCmd>, Vec<StatusFragment>),
}

/// What draining the panes found: `Eof` means the last pane is gone and the session ends
/// after one final frame.
#[derive(PartialEq, Eq)]
enum Drained {
    More,
    Eof,
}

impl Session {
    fn new(size: Size) -> anyhow::Result<Self> {
        let has_status = false;
        let first = PaneId::from(FIRST_PANE);
        let layout = PaneLayout::new(size, has_status, first);
        let mut shells = Shells::default();
        shells.open(
            first,
            PtyHandle::default_shell_command(),
            pane_size(&layout, first)?,
        )?;
        Ok(Self {
            size,
            layout,
            has_status,
            shells,
            next_pane: FIRST_PANE + 1,
            dirty: true,
            last_frame_output: (Vec::new(), Vec::new()),
        })
    }

    /// Resizes every pane that is big enough to be shown; the rest keep their last size (§6).
    fn fit(&mut self) -> anyhow::Result<()> {
        let Self { shells, layout, .. } = self;
        shells.fit(layout.showable())
    }

    /// A geometry change, whether from a real terminal resize or the status row
    /// appearing/disappearing: recompute the layout, resize the panes to match, and tell the
    /// Lua layer about it (§4.8).
    fn apply_geometry(&mut self, lua: &LuaSubsystem) -> anyhow::Result<()> {
        self.layout.resurface(self.size, self.has_status);
        self.fit()?;
        lua.dispatch_event(&LuaEvent::Resized {
            size: self.layout.content,
        });
        self.dirty = true;
        Ok(())
    }

    /// At most `MAX_CHUNKS_PER_FRAME` chunks per pane, so a flooding child cannot starve
    /// input or the other panes.
    fn drain_panes(&mut self, lua: &LuaSubsystem) -> anyhow::Result<Drained> {
        let drained = self.shells.drain(MAX_CHUNKS_PER_FRAME, |pane, bytes| {
            lua.dispatch_event(&LuaEvent::PaneOutput { pane, bytes });
        })?;
        if drained.output {
            self.dirty = true;
        }
        // The last pane's final output and its EOF arrive together; leaving it open for one
        // more frame is what renders that output before the session ends (M0a's guarantee).
        // Dropping the session after that frame is what reaps the children.
        if !drained.closed.is_empty() && drained.closed.len() == self.shells.len() {
            return Ok(Drained::Eof);
        }
        for pane in drained.closed {
            self.close_pane(pane, lua)?;
        }
        Ok(Drained::More)
    }

    /// A pane's child ended: reap it, give its space to its sibling, and move focus if it
    /// was the focused one. The other panes keep running.
    fn close_pane(&mut self, pane: PaneId, lua: &LuaSubsystem) -> anyhow::Result<()> {
        self.shells.close(pane);
        let before = self.layout.focus();
        let after = self.layout.close(pane);
        self.fit()?;
        self.republish_geometry(lua);
        lua.dispatch_event(&LuaEvent::PaneClosed { pane });
        if after != before
            && let Some(focus) = after
        {
            lua.dispatch_event(&LuaEvent::FocusChanged { pane: focus });
        }
        self.dirty = true;
        Ok(())
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
                    // A key nobody consumed belongs to the focused pane and to no other.
                    // With no pane focused there is no shell left to type into.
                    if outcome == KeyOutcome::Fallthrough
                        && let Some(focus) = self.layout.focus()
                        && let Some(bytes) = encode_key(key)
                    {
                        self.shells.write(focus, &bytes)?;
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
    fn drain_ops(
        &mut self,
        lua: &LuaSubsystem,
        notifications: &mut Notifications,
    ) -> anyhow::Result<bool> {
        let mut quit = false;
        let mut splits = 0usize;
        for (id, op) in lua.drain_ops() {
            let tag = lua.sources().name(id).unwrap_or_else(|| id.to_string());
            match op {
                Op::Quit => quit = true,
                Op::Log { level, text } => logging::log(level, &tag, &text),
                Op::Notify { level, text } => {
                    notifications.push(level, &tag, &text);
                    logging::log(level, &tag, &text);
                }
                Op::PaneSplit { dir } if splits >= MAX_SPLITS_PER_FRAME => logging::log(
                    Level::Error,
                    &tag,
                    &format!(
                        "pane.split('{dir}') refused: at most {MAX_SPLITS_PER_FRAME} splits per frame"
                    ),
                ),
                Op::PaneSplit { dir } => {
                    splits += 1;
                    self.apply_split(lua, &tag, dir)?;
                }
                Op::PaneFocus { pane } => self.apply_focus(lua, &tag, pane),
            }
        }
        Ok(quit)
    }

    fn apply_split(
        &mut self,
        lua: &LuaSubsystem,
        tag: &str,
        dir: neotmux_core::proto::SplitDir,
    ) -> anyhow::Result<()> {
        let new = PaneId::from(self.next_pane);
        if let Err(refused) = self.layout.split(dir, new) {
            logging::log(
                Level::Error,
                tag,
                &format!("pane.split('{dir}') refused: {refused}"),
            );
            return Ok(());
        }

        let started = pane_size(&self.layout, new).and_then(|size| {
            self.shells
                .open(new, PtyHandle::default_shell_command(), size)
        });
        if let Err(err) = started {
            // The tree already holds a pane with nothing running in it; undo it rather than
            // leave a pane that can never draw or be typed into.
            self.layout.close(new);
            logging::log(
                Level::Error,
                tag,
                &format!("pane.split('{dir}') could not start a shell: {err:#}"),
            );
            return Ok(());
        }

        self.next_pane += 1;
        self.fit()?;
        self.republish_geometry(lua);
        lua.dispatch_event(&LuaEvent::PaneOpened { pane: new });
        lua.dispatch_event(&LuaEvent::FocusChanged { pane: new });
        self.dirty = true;
        Ok(())
    }

    fn apply_focus(&mut self, lua: &LuaSubsystem, tag: &str, pane: PaneId) {
        if !self.layout.set_focus(pane) {
            logging::log(
                Level::Error,
                tag,
                &format!("pane.focus({pane}) refused: no such pane"),
            );
            return;
        }
        self.republish_geometry(lua);
        lua.dispatch_event(&LuaEvent::FocusChanged { pane });
        self.dirty = true;
    }

    /// Hands Lua this frame's geometry. Called once before the loop as well, because keys
    /// are pumped before the tick and a plugin asking `pane.list()` from `on_key` must not
    /// see an empty session on the very first key.
    fn publish_geometry(&self, lua: &LuaSubsystem) {
        lua.begin_frame(
            self.layout.epoch(),
            self.layout.content,
            self.layout.panes(),
            self.layout.focus(),
        );
    }

    /// The tree changed mid-frame. A `pane_opened` handler calling `pane.list()` must see the
    /// pane it is being told about, and a `pane_closed` handler must not see the dead one —
    /// so the geometry catches up before any of those events is dispatched.
    fn republish_geometry(&self, lua: &LuaSubsystem) {
        lua.republish_geometry(
            self.layout.content,
            self.layout.panes(),
            self.layout.focus(),
        );
    }

    /// Whether the status row just appeared or disappeared, which changes the panes' size.
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
        self.shells.snapshot();
        notifications.expire(Instant::now());

        let shells = &self.shells;
        let pane_layout = &self.layout;
        let has_status = self.has_status;
        terminal
            .draw(|frame| {
                let area = frame.area();
                let (pane_area, status_area) = layout(area, has_status);
                let focus = pane_layout.focus();
                let mut focused_cell = None;
                for (pane, rect) in pane_layout.showable() {
                    let Some(grid) = shells.grid(pane) else {
                        continue;
                    };
                    let cell = Rect {
                        x: pane_area.x + rect.x,
                        y: pane_area.y + rect.y,
                        width: rect.w,
                        height: rect.h,
                    }
                    .intersection(pane_area);
                    draw_grid(frame, cell, grid);
                    if focus == Some(pane) {
                        focused_cell = Some((cell, grid));
                    }
                }
                if let Some(position) =
                    focused_cell.and_then(|(cell, grid)| cursor_position(cell, grid))
                {
                    frame.set_cursor_position(position);
                }
                overlay::apply(frame, pane_area, cmds, &pane_layout.rects);
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

/// A pane with no rect is a bug in the layout, not a 0x0 pane: spawning a PTY at the default
/// size would turn it into a terminal nothing can draw on, which is far harder to find.
fn pane_size(layout: &PaneLayout, pane: PaneId) -> anyhow::Result<Size> {
    let rect = layout
        .rects
        .get(&pane)
        .copied()
        .with_context(|| format!("{pane} has no rect in the layout"))?;
    Ok(Size {
        rows: rect.h,
        cols: rect.w,
    })
}

pub fn run(
    terminal: &mut DefaultTerminal,
    shutdown: &ShutdownSignal,
    lua: &LuaSubsystem,
    notifications: &mut Notifications,
) -> anyhow::Result<Option<i32>> {
    let mut session = Session::new(terminal_size(terminal)?)?;
    session.publish_geometry(lua);

    #[cfg(debug_assertions)]
    if std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        if let Some(signal) = shutdown.raised() {
            return Ok(Some(signal));
        }

        let frame_end = Instant::now() + FRAME;
        let drained = session.drain_panes(lua)?;

        if drained == Drained::More
            && let Some(signal) = session.pump_input(lua, shutdown, frame_end)?
        {
            return Ok(Some(signal));
        }

        session.publish_geometry(lua);
        lua.dispatch_tick();

        if session.drain_ops(lua, notifications)? {
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

    Ok(None)
}

#[cfg(test)]
mod tests {
    use neotmux_term::pty::CommandBuilder;
    use neotmux_term::state::{MIN_GRID, TermState};

    use super::*;

    #[test]
    fn a_zero_size_is_clamped_the_same_for_state_and_pty() {
        let raw = Size { rows: 0, cols: 0 };
        let clamped = clamp_to_min_grid(raw);
        assert_eq!(clamped, MIN_GRID);

        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "true"]);
        let state = TermState::new(clamped, 0);
        let mut pty = PtyHandle::spawn(cmd, clamped).expect("spawn pty");

        assert_eq!(state.size(), clamped);
        assert_eq!(pty.size().expect("pty size"), clamped);

        pty.wait().ok();
    }

    #[test]
    fn resizing_to_zero_is_clamped_the_same_for_state_and_pty() {
        let initial = Size { rows: 24, cols: 80 };
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "true"]);
        let mut state = TermState::new(initial, 0);
        let pty = PtyHandle::spawn(cmd, initial).expect("spawn pty");

        let clamped = clamp_to_min_grid(Size { rows: 0, cols: 0 });
        state.resize(clamped);
        pty.resize(clamped).expect("resize pty");

        assert_eq!(state.size(), clamped);
        assert_eq!(pty.size().expect("pty size"), clamped);
    }

    #[test]
    fn the_first_pane_is_the_whole_surface() {
        let size = Size { rows: 24, cols: 80 };
        let layout = PaneLayout::new(size, false, PaneId::from(FIRST_PANE));
        assert_eq!(
            pane_size(&layout, PaneId::from(FIRST_PANE)).expect("the first pane has a rect"),
            size
        );
    }

    /// Unreachable today, and that is the point: if the layout ever loses a pane's rect it
    /// has to be an error rather than a 0x0 PTY nobody can see is wrong.
    #[test]
    fn a_pane_with_no_rect_is_an_error_not_a_zero_sized_terminal() {
        let layout = PaneLayout::new(Size { rows: 24, cols: 80 }, false, PaneId::from(FIRST_PANE));
        assert!(pane_size(&layout, PaneId::from(99)).is_err());
    }
}

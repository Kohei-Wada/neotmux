use std::time::Instant;

use anyhow::Context as _;
use neotmux_lua::ops::Op;
use neotmux_lua::{Event as LuaEvent, Geometry, KeyOutcome, LuaSubsystem, Window as LuaWindow};
use neotmux_proto::{DrawCmd, Level, PaneId, Rect as PRect, Reserved, Size, WindowId};
use neotmux_term::pty::PtyHandle;
use neotmux_term::state::clamp_to_min_grid;
use neotmux_term::wakeup::Waker;

use crate::input::{encode_key, key_spec};
use crate::logging;
use crate::notify::Notifications;
use crate::paneinfo::PaneInfoCache;
use crate::procinfo;
use crate::shells::Shells;
use crate::windows::{Shown, Surface, Windows};
use neotmux_proto::frame::{Frame, PaneFrame};
use neotmux_proto::keys::Key;

const MAX_CHUNKS_PER_FRAME: usize = 32;

/// How many new shells one frame will fork — a `pane.split` or a `window.create`, counted
/// together because the cost is the same one. It is host work a plugin buys from outside its
/// measured tick — an op pushed in the tick, a shell fork/exec'd after it — so M1b's promise
/// only holds if the number of them a frame can be made to do is bounded.
///
/// A conservative fixed floor, not a formula to re-run, and compile-time on purpose. It is
/// not derived from `tick_budget_ms`: that budget is per source per frame, so k sources can
/// claim 8k ms of a 16 ms frame and there is no remainder to divide; `tick_budget_ms = 0`
/// switches the budget off altogether; and a cap read from an option a plugin sets would let
/// it widen its own allowance. The measurement behind 3: one split costs about 1.3 ms
/// (release, 24x40 pane: openpty, fork, exec, terminal state), so three of them spend a few
/// milliseconds of the frame on something a session asks for a handful of times.
const MAX_NEW_SHELLS_PER_FRAME: usize = 3;

/// The state one frame carries into the next. Every step of a frame is a method on it, so
/// the loop driving the session reads as the order of the steps and nothing else.
pub struct Session {
    /// What every window is laid out against: the whole terminal, the bands reserved at
    /// its edges, and the gutter between panes.
    surface: Surface,
    /// Every pane tree, and which one is being shown.
    windows: Windows,
    /// Every pane's child, the hidden windows' included: a shell nobody is looking at
    /// goes on running, and its output goes on being read.
    shells: Shells,
    /// What each pane is running, sampled on its own cadence rather than per frame.
    pane_info: PaneInfoCache,
    dirty: bool,
    last_frame_output: Vec<DrawCmd>,
}

/// What draining the panes found: `Eof` means the last pane is gone and the session ends
/// after one final frame.
#[derive(PartialEq, Eq)]
enum Drained {
    More,
    Eof,
}

impl Session {
    pub fn new(size: Size, lua: &LuaSubsystem, waker: Waker) -> anyhow::Result<Self> {
        let surface = Surface {
            size,
            reserved: lua.reserved(),
            gutter: lua.opt().pane_gutter,
        };
        let (windows, first) = Windows::new(lua.reserve_window_id(), surface);
        lua.lay_out_reservations(size, windows.current().applied_bands());
        let mut shells = Shells::new(waker, lua.terminals());
        shells.open(
            first,
            PtyHandle::default_shell_command(),
            pane_size(windows.rect(first), first)?,
        )?;
        let mut session = Self {
            surface,
            windows,
            shells,
            pane_info: PaneInfoCache::new(),
            dirty: true,
            last_frame_output: Vec::new(),
        };
        session.refresh_pane_info(lua);
        // Before the first key rather than at the first frame: keys are pumped before the
        // tick, and a plugin asking `pane.list()` from `on_key` must not see an empty
        // session on the very first one.
        session.publish_geometry(lua);
        Ok(session)
    }

    /// Resizes every pane of the shown window that is big enough to be shown; the rest
    /// keep their last size (§6). A hidden window's panes are not here: they keep the
    /// size they had, and are refitted on the way back in.
    fn fit(&mut self) -> anyhow::Result<()> {
        let showable: Vec<(PaneId, PRect)> = self.windows.current().showable().collect();
        self.shells.fit(showable.into_iter())
    }

    /// A geometry change, whether from a real terminal resize, a band being reserved or
    /// released, or the gutter changing: recompute the layout, hand the bands their place in
    /// it, resize the panes to match, and tell the Lua layer about it (§4.8).
    fn apply_geometry(&mut self, lua: &LuaSubsystem) -> anyhow::Result<()> {
        self.windows.resurface(self.surface);
        lua.lay_out_reservations(self.surface.size, self.windows.current().applied_bands());
        self.fit()?;
        lua.dispatch_event(&LuaEvent::Resized {
            size: self.windows.current().content,
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
    /// was the focused one. The other panes keep running. The pane may belong to a window
    /// nobody is looking at, and closing the last pane of one closes the window too.
    fn close_pane(&mut self, pane: PaneId, lua: &LuaSubsystem) -> anyhow::Result<()> {
        self.shells.close(pane);
        self.pane_info.invalidate();
        let was_shown = self.windows.current().rects.contains_key(&pane);
        let before = self.windows.current().focus();
        let Some(closed) = self.windows.close_pane(pane, self.surface) else {
            return Ok(());
        };
        // The last pane anywhere never reaches here: it is `Drained::Eof`, which ends the
        // session after one more frame so that pane's final output is drawn.
        if self.windows.is_empty() {
            return Ok(());
        }
        self.fit()?;
        self.republish_geometry(lua);
        lua.dispatch_event(&LuaEvent::PaneClosed { pane });
        if closed.window_closed {
            lua.dispatch_event(&LuaEvent::WindowClosed {
                window: closed.window,
            });
        }
        if let Some(window) = closed.switched_to {
            lua.dispatch_event(&LuaEvent::WindowChanged { window });
        }
        let after = self.windows.current().focus();
        if was_shown
            && after != before
            && let Some(focus) = after
        {
            lua.dispatch_event(&LuaEvent::FocusChanged { pane: focus });
        }
        self.dirty = true;
        Ok(())
    }

    /// A key the session was given. One nobody bound belongs to the focused pane and to no
    /// other; with no pane focused there is no shell left to type into.
    pub fn key(&mut self, lua: &LuaSubsystem, key: Key) -> anyhow::Result<()> {
        if lua.dispatch_key(&key_spec(&key)) == KeyOutcome::Fallthrough
            && let Some(focus) = self.windows.current().focus()
            && let Some(bytes) = encode_key(key)
        {
            self.shells.write(focus, &bytes)?;
        }
        Ok(())
    }

    /// The terminal changed size. Nothing arrives on the terminal to say so — `SIGWINCH` is
    /// the whole of the announcement — so the new size is asked for rather than read.
    pub fn resize(&mut self, size: Size, lua: &LuaSubsystem) -> anyhow::Result<()> {
        self.surface.size = clamp_to_min_grid(size);
        self.apply_geometry(lua)
    }

    /// Every op a source raised this frame, applied in the order they were pushed (§4.6).
    /// Returns whether one of them asked to quit.
    fn drain_ops(
        &mut self,
        lua: &LuaSubsystem,
        notifications: &mut Notifications,
    ) -> anyhow::Result<bool> {
        let mut quit = false;
        let mut spawned = 0usize;
        for (id, op) in lua.drain_ops() {
            let tag = lua.sources().name(id).unwrap_or_else(|| id.to_string());
            match op {
                Op::Quit => quit = true,
                Op::Log { level, text } => logging::log(level, &tag, &text),
                Op::Notify { level, text } => {
                    notifications.push(level, &tag, &text);
                    logging::log(level, &tag, &text);
                }
                Op::PaneSplit { dir } if spawned >= MAX_NEW_SHELLS_PER_FRAME => logging::log(
                    Level::Error,
                    &tag,
                    &format!(
                        "pane.split('{dir}') refused: at most {MAX_NEW_SHELLS_PER_FRAME} new \
                         shells per frame"
                    ),
                ),
                Op::PaneSplit { dir } => {
                    spawned += 1;
                    self.apply_split(lua, &tag, dir)?;
                }
                Op::PaneFocus { pane } => self.apply_focus(lua, &tag, pane),
                Op::PaneResize { dir, cells } => self.apply_resize(lua, &tag, dir, cells)?,
                Op::WindowCreate { .. } if spawned >= MAX_NEW_SHELLS_PER_FRAME => logging::log(
                    Level::Error,
                    &tag,
                    &format!(
                        "window.create() refused: at most {MAX_NEW_SHELLS_PER_FRAME} new \
                             shells per frame"
                    ),
                ),
                Op::WindowCreate { window } => {
                    spawned += 1;
                    self.apply_window_create(lua, &tag, window)?;
                }
                Op::WindowFocus { window } => self.apply_window_focus(lua, &tag, window)?,
            }
        }
        Ok(quit)
    }

    fn apply_split(
        &mut self,
        lua: &LuaSubsystem,
        tag: &str,
        dir: neotmux_proto::SplitDir,
    ) -> anyhow::Result<()> {
        let new = self.windows.take_pane_id();
        if let Err(refused) = self.windows.current_mut().split(dir, new) {
            logging::log(
                Level::Error,
                tag,
                &format!("pane.split('{dir}') refused: {refused}"),
            );
            return Ok(());
        }

        let started = pane_size(self.windows.rect(new), new).and_then(|size| {
            self.shells
                .open(new, PtyHandle::default_shell_command(), size)
        });
        if let Err(err) = started {
            // The tree already holds a pane with nothing running in it; undo it rather than
            // leave a pane that can never draw or be typed into.
            self.windows.current_mut().close(new);
            logging::log(
                Level::Error,
                tag,
                &format!("pane.split('{dir}') could not start a shell: {err:#}"),
            );
            return Ok(());
        }

        self.pane_info.invalidate();
        self.fit()?;
        self.republish_geometry(lua);
        lua.dispatch_event(&LuaEvent::PaneOpened { pane: new });
        lua.dispatch_event(&LuaEvent::FocusChanged { pane: new });
        self.dirty = true;
        Ok(())
    }

    /// A new window, with one pane and one shell in it. It is not shown: `window.focus`
    /// is a separate op, so `focus(create())` in one tick opens it and switches to it in that
    /// order while `create()` alone leaves it running out of sight.
    fn apply_window_create(
        &mut self,
        lua: &LuaSubsystem,
        tag: &str,
        window: WindowId,
    ) -> anyhow::Result<()> {
        let first = self.windows.create(window, self.surface);
        let started = pane_size(self.windows.rect(first), first).and_then(|size| {
            self.shells
                .open(first, PtyHandle::default_shell_command(), size)
        });
        if let Err(err) = started {
            // A window whose only pane can never draw is worse than no window: undo it
            // the same way a split that could not start a shell is undone.
            self.windows.close_pane(first, self.surface);
            logging::log(
                Level::Error,
                tag,
                &format!("window.create() could not start a shell: {err:#}"),
            );
            return Ok(());
        }
        // The pane list is unchanged — the new pane is not in the shown window — but the
        // window list is, and a `window_opened` handler must see what it is told about.
        self.republish_geometry(lua);
        lua.dispatch_event(&LuaEvent::WindowOpened { window });
        self.dirty = true;
        Ok(())
    }

    /// Shows another window: its panes are caught up with a surface that may have changed
    /// while they were hidden, then resized to it.
    fn apply_window_focus(
        &mut self,
        lua: &LuaSubsystem,
        tag: &str,
        window: WindowId,
    ) -> anyhow::Result<()> {
        match self.windows.show(window, self.surface) {
            Shown::NoSuchWindow => {
                logging::log(
                    Level::Error,
                    tag,
                    &format!("window.focus({window}) refused: no such window"),
                );
                return Ok(());
            }
            Shown::AlreadyCurrent => return Ok(()),
            Shown::Switched => {}
        }
        self.pane_info.invalidate();
        self.fit()?;
        self.republish_geometry(lua);
        lua.dispatch_event(&LuaEvent::WindowChanged { window });
        if let Some(focus) = self.windows.current().focus() {
            lua.dispatch_event(&LuaEvent::FocusChanged { pane: focus });
        }
        self.dirty = true;
        Ok(())
    }

    /// A refusal is logged against whoever asked, as a refused split is: the op is applied
    /// after the tick, so there is nobody left to answer.
    fn apply_resize(
        &mut self,
        lua: &LuaSubsystem,
        tag: &str,
        dir: neotmux_proto::Dir,
        cells: u16,
    ) -> anyhow::Result<()> {
        if let Err(refused) = self.windows.current_mut().resize(dir, cells) {
            logging::log(
                Level::Error,
                tag,
                &format!("pane.resize('{dir}', {cells}) refused: {refused}"),
            );
            return Ok(());
        }
        self.fit()?;
        self.republish_geometry(lua);
        self.dirty = true;
        Ok(())
    }

    fn apply_focus(&mut self, lua: &LuaSubsystem, tag: &str, pane: PaneId) {
        if !self.windows.current_mut().set_focus(pane) {
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

    /// Samples what each pane is running, if the cadence says it is time, and hands the
    /// result to Lua. Reading `/proc` for every pane on every frame is the syscall storm this
    /// exists to avoid, so most frames do nothing here.
    ///
    /// Every pane the session holds, hidden windows included: what a pane in the window you
    /// are not looking at is running is exactly the question a sidebar exists to answer, and
    /// the cadence rather than the number of panes is what bounds the bill.
    fn refresh_pane_info(&mut self, lua: &LuaSubsystem) {
        let shells = &self.shells;
        let panes = self.windows.all_panes();
        let refreshed = self.pane_info.refresh(Instant::now(), panes, |pane| {
            match shells.foreground(pane) {
                Some(pid) => procinfo::read(pid),
                None => Default::default(),
            }
        });
        if refreshed {
            lua.publish_pane_info(self.pane_info.snapshot());
        }
    }

    /// Hands Lua this frame's geometry.
    fn publish_geometry(&self, lua: &LuaSubsystem) {
        lua.begin_frame(self.windows.current().epoch(), self.geometry());
    }

    /// The tree changed mid-frame. A `pane_opened` handler calling `pane.list()` must see the
    /// pane it is being told about, and a `pane_closed` handler must not see the dead one —
    /// so the geometry catches up before any of those events is dispatched.
    fn republish_geometry(&self, lua: &LuaSubsystem) {
        lua.republish_geometry(self.geometry());
    }

    /// The session as Lua reads it: the shown window's panes and rects, and every window
    /// with the panes it holds.
    fn geometry(&self) -> Geometry {
        let layout = self.windows.current();
        Geometry {
            size: layout.content,
            panes: layout.panes(),
            focus: layout.focus(),
            windows: self
                .windows
                .panes()
                .into_iter()
                .map(|(id, panes)| LuaWindow { id, panes })
                .collect(),
            window: Some(self.windows.current_id()),
        }
    }

    /// Whether a band was reserved or released, which changes the panes' size. Sampled here,
    /// at the frame boundary, and nowhere else: a `neotmux.reserve` from inside a tick would
    /// otherwise change the content size — and with it the epoch — under a frame that had
    /// already committed its drawing against the old one.
    fn reserved_changed(&mut self, reserved: Reserved) -> bool {
        if reserved == self.surface.reserved {
            return false;
        }
        self.surface.reserved = reserved;
        true
    }

    /// Whether the gutter between panes just changed, which changes the panes' size. It is an
    /// option like any other, so a plugin can set it after the frame it was first read on.
    fn gutter_changed(&mut self, gutter: u16) -> bool {
        if gutter == self.surface.gutter {
            return false;
        }
        self.surface.gutter = gutter;
        true
    }

    /// What this frame shows. Producing it is the session's work; putting it on a screen is
    /// not, and nothing below this line knows what kind of screen there is.
    fn frame(&mut self, notifications: &mut Notifications) -> Frame<'_> {
        // The shown window's panes only: a hidden pane's terminal goes on taking its
        // child's output, but copying its grid out for a frame nobody draws it in would put
        // every window on every frame's bill.
        let shown: Vec<PaneId> = self.windows.current().rects.keys().copied().collect();
        self.shells.snapshot(shown.into_iter());
        notifications.expire(Instant::now());
        self.dirty = false;

        let shells = &self.shells;
        let layout = self.windows.current();
        Frame {
            size: self.surface.size,
            panes: layout
                .showable()
                .filter_map(|(pane, rect)| {
                    Some(PaneFrame {
                        pane,
                        rect,
                        grid: shells.grid(pane)?,
                    })
                })
                .collect(),
            rects: &layout.rects,
            cursor: layout.focus(),
            overlay: &self.last_frame_output,
            notices: notifications.visible(),
        }
    }

    /// One pass of the session's own work: the panes' output, the tick, the ops it raised,
    /// and the frame that came of it. The frame is `None` when nothing changed and the last
    /// one still stands.
    pub fn advance(
        &mut self,
        lua: &LuaSubsystem,
        notifications: &mut Notifications,
    ) -> anyhow::Result<Step<'_>> {
        let drained = self.drain_panes(lua)?;

        self.refresh_pane_info(lua);
        self.publish_geometry(lua);
        lua.dispatch_tick();

        if self.drain_ops(lua, notifications)? {
            return Ok(Step {
                frame: None,
                end: Some(End::Quit),
            });
        }

        let frame_output = lua.frame_output();
        // After the frame's output is in hand, never before: both of these change the size of
        // the surface, and this frame's buffers were committed against the old one.
        let reserved_changed = self.reserved_changed(lua.reserved());
        let gutter_changed = self.gutter_changed(lua.opt().pane_gutter);
        if reserved_changed || gutter_changed {
            self.apply_geometry(lua)?;
        }

        if frame_output != self.last_frame_output {
            self.dirty = true;
        }
        self.last_frame_output = frame_output;
        // A live notification needs a later frame just to notice it expired
        // and stop being drawn, even if nothing else changed.
        if !notifications.is_empty() {
            self.dirty = true;
        }

        let eof = drained == Drained::Eof;
        let frame = if self.dirty || eof {
            Some(self.frame(notifications))
        } else {
            None
        };
        Ok(Step {
            frame,
            end: eof.then_some(End::Eof),
        })
    }
}

/// What one pass of the session produced.
pub struct Step<'a> {
    /// The frame to show, if this pass changed anything.
    pub frame: Option<Frame<'a>>,
    /// Why the session is over, if it is.
    pub end: Option<End>,
}

/// The two ways a session ends of its own accord: a source asked it to, or its last pane's
/// child did. A signal is neither — it ends the loop rather than the session.
#[derive(PartialEq, Eq)]
pub enum End {
    Quit,
    Eof,
}
/// A pane with no rect is a bug in the layout, not a 0x0 pane: spawning a PTY at the default
/// size would turn it into a terminal nothing can draw on, which is far harder to find.
fn pane_size(rect: Option<PRect>, pane: PaneId) -> anyhow::Result<Size> {
    let rect = rect.with_context(|| format!("{pane} has no rect in the layout"))?;
    Ok(Size {
        rows: rect.h,
        cols: rect.w,
    })
}

#[cfg(test)]
mod tests {
    use neotmux_term::pty::CommandBuilder;
    use neotmux_term::state::{MIN_GRID, TermState};
    use neotmux_term::wakeup::wakeup;

    use super::*;

    #[test]
    fn a_zero_size_is_clamped_the_same_for_state_and_pty() {
        let raw = Size { rows: 0, cols: 0 };
        let clamped = clamp_to_min_grid(raw);
        assert_eq!(clamped, MIN_GRID);

        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "true"]);
        let state = TermState::new(clamped, 0);
        let (_wakeup, waker) = wakeup().expect("wakeup");
        let mut pty = PtyHandle::spawn(cmd, clamped, waker).expect("spawn pty");

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
        let (_wakeup, waker) = wakeup().expect("wakeup");
        let pty = PtyHandle::spawn(cmd, initial, waker).expect("spawn pty");

        let clamped = clamp_to_min_grid(Size { rows: 0, cols: 0 });
        state.resize(clamped);
        pty.resize(clamped).expect("resize pty");

        assert_eq!(state.size(), clamped);
        assert_eq!(pty.size().expect("pty size"), clamped);
    }

    #[test]
    fn the_first_pane_is_the_whole_surface() {
        let size = Size { rows: 24, cols: 80 };
        let surface = Surface {
            size,
            reserved: Reserved::default(),
            gutter: 0,
        };
        let (windows, first) = Windows::new(WindowId::from(1), surface);
        assert_eq!(
            pane_size(windows.rect(first), first).expect("the first pane has a rect"),
            size
        );
    }

    /// Unreachable today, and that is the point: if the layout ever loses a pane's rect it
    /// has to be an error rather than a 0x0 PTY nobody can see is wrong.
    #[test]
    fn a_pane_with_no_rect_is_an_error_not_a_zero_sized_terminal() {
        assert!(pane_size(None, PaneId::from(99)).is_err());
    }
}

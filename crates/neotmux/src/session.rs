use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use anyhow::Context as _;
use neotmux_proto::msg::{Attach, End, SessionOp, Step, Tick, Tock};
use neotmux_proto::{
    DrawCmd, Event, Geometry, Level, PaneId, PaneInfo, Rect as PRect, Reserved, Size,
    Window as LuaWindow, WindowId,
};
use neotmux_term::pty::PtyHandle;
use neotmux_term::state::clamp_to_min_grid;
use neotmux_term::terminals::PaneTerminals;
use neotmux_term::wakeup::Waker;

use crate::input::encode_key;
use crate::logging;
use crate::paneinfo::PaneInfoCache;
use crate::procinfo;
use crate::shells::Shells;
use crate::windows::{Shown, Surface, Windows};
use neotmux_proto::frame::{Frame, Notice, PaneFrame};
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
    last_overlay: Vec<DrawCmd>,
    last_notices: Vec<Notice>,
    /// What has happened to the session since the client was last told, in order. A batch
    /// with one geometry attached would be a lie: a `pane_opened` handler must see the pane
    /// it is being told about, so the geometry moves between the events exactly as it does
    /// when both halves are one process.
    steps: Vec<Step>,
    /// Whether any plugin is subscribed to `pane_output`. Nothing is, almost always, and a
    /// pane's bytes are the one payload worth not sending.
    wants_pane_output: bool,
    /// The last pane's child is gone. The session ends after one more frame, which is what
    /// draws that pane's final output.
    ended: bool,
    terminals: Rc<PaneTerminals>,
}

/// What draining the panes found: `Eof` means the last pane is gone and the session ends
/// after one final frame.
#[derive(PartialEq, Eq)]
enum Drained {
    More,
    Eof,
}

impl Session {
    /// Starts the session a client has just asked for. The size, the reservations, the
    /// gutter and the first window's id all come from the client, because all four are the
    /// client's to know: the terminal is its, and window ids are the Lua layer's.
    pub fn new(attach: &Attach, waker: Waker) -> anyhow::Result<Self> {
        let surface = Surface {
            size: clamp_to_min_grid(attach.size),
            reserved: attach.reserved,
            gutter: attach.gutter,
        };
        let (windows, first) = Windows::new(attach.first_window, surface);
        let terminals = Rc::new(PaneTerminals::new());
        let mut shells = Shells::new(waker, terminals.clone());
        // The session's own directory: the server is a grandchild of the directory neotmux
        // was started from, and `start_dir` cannot be asked for it yet — there is no pane
        // to point at. The pane that is about to be made is the session's first.
        let mut cmd = PtyHandle::default_shell_command();
        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }
        shells.open(first, cmd, pane_size(windows.rect(first), first)?)?;
        let mut session = Self {
            surface,
            windows,
            shells,
            pane_info: PaneInfoCache::new(),
            dirty: true,
            last_overlay: Vec::new(),
            last_notices: Vec::new(),
            steps: Vec::new(),
            wants_pane_output: attach.wants_pane_output,
            ended: false,
            terminals,
        };
        session.lay_out_reservations();
        // Before the first key rather than at the first tick: a plugin asking `pane.list()`
        // from `on_key` must not see an empty session on the very first key.
        session.publish_geometry();
        Ok(session)
    }

    /// The pane terminals, which answer the reads the Lua layer asks across the socket.
    pub fn terminals(&self) -> &Rc<PaneTerminals> {
        &self.terminals
    }

    fn lay_out_reservations(&mut self) {
        self.steps.push(Step::Bands {
            surface: self.surface.size,
            applied: self.windows.current().applied_bands(),
        });
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
    fn apply_geometry(&mut self) -> anyhow::Result<()> {
        self.windows.resurface(self.surface);
        self.lay_out_reservations();
        self.fit()?;
        let size = self.windows.current().content;
        self.steps.push(Step::Event(Event::Resized { size }));
        self.dirty = true;
        Ok(())
    }

    /// At most `MAX_CHUNKS_PER_FRAME` chunks per pane, so a flooding child cannot starve
    /// input or the other panes.
    fn drain_panes(&mut self) -> anyhow::Result<Drained> {
        let wanted = self.wants_pane_output;
        let steps = &mut self.steps;
        let drained = self.shells.drain(MAX_CHUNKS_PER_FRAME, |pane, bytes| {
            if wanted {
                steps.push(Step::Event(Event::PaneOutput { pane, bytes }));
            }
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
            self.close_pane(pane)?;
        }
        Ok(Drained::More)
    }

    /// A pane's child ended: reap it, give its space to its sibling, and move focus if it
    /// was the focused one. The other panes keep running. The pane may belong to a window
    /// nobody is looking at, and closing the last pane of one closes the window too.
    fn close_pane(&mut self, pane: PaneId) -> anyhow::Result<()> {
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
        self.republish_geometry();
        self.steps.push(Step::Event(Event::PaneClosed { pane }));
        if closed.window_closed {
            self.steps.push(Step::Event(Event::WindowClosed {
                window: closed.window,
            }));
        }
        if let Some(window) = closed.switched_to {
            self.steps
                .push(Step::Event(Event::WindowChanged { window }));
        }
        let after = self.windows.current().focus();
        if was_shown
            && after != before
            && let Some(focus) = after
        {
            self.steps
                .push(Step::Event(Event::FocusChanged { pane: focus }));
        }
        self.dirty = true;
        Ok(())
    }

    /// A key nobody on the client bound. It belongs to the focused pane and to no other;
    /// with no pane focused there is no shell left to type into.
    pub fn key(&mut self, key: Key) -> anyhow::Result<()> {
        if let Some(focus) = self.windows.current().focus()
            && let Some(bytes) = encode_key(key)
        {
            self.shells.write(focus, &bytes)?;
        }
        Ok(())
    }

    /// The terminal changed size. Nothing arrives on the terminal to say so — `SIGWINCH` is
    /// the whole of the announcement — so the new size is asked for rather than read.
    pub fn resize(&mut self, size: Size) -> anyhow::Result<()> {
        self.surface.size = clamp_to_min_grid(size);
        self.apply_geometry()
    }

    /// A new client taking over a session that is already running. `Attach`'s fields are
    /// values to apply here rather than arguments to build from: the terminal is this
    /// client's, and so are the bands, the gutter and the subscriptions, because the Lua
    /// layer that declared them is new.
    ///
    /// `first_window` is ignored — the windows already have ids.
    pub fn adopt(&mut self, attach: &Attach) -> anyhow::Result<()> {
        self.surface.size = clamp_to_min_grid(attach.size);
        self.surface.reserved = attach.reserved;
        self.surface.gutter = attach.gutter;
        self.wants_pane_output = attach.wants_pane_output;
        // What happened while nobody was looking is not news for a client that was not
        // there: geometry is how a plugin learns what exists, events how it learns what
        // changed, and everything here is the former.
        self.steps.clear();
        self.apply_geometry()?;
        // `apply_geometry` raises `Resized`, but a client that was never told a previous size
        // has nothing to be resized from (§4: "No events are replayed").
        self.steps.retain(|step| !matches!(step, Step::Event(_)));
        self.publish_geometry();
        // Nothing this client has been shown is on its screen, so the first frame is not
        // allowed to be "nothing changed".
        self.last_overlay.clear();
        self.last_notices.clear();
        self.dirty = true;
        Ok(())
    }

    /// Every op the tick raised, applied in the order they were pushed (§4.6). Returns which
    /// end, if any, one of them asked for — the first one asked, since once the conversation
    /// is over there is nothing left for a later op to change.
    ///
    /// `log` and `notify` are not here: they never leave the client, which is where the
    /// source that raised them is. A refusal does come back here, and is logged against
    /// whoever asked — the op is applied after the tick, so there is nobody left to answer.
    fn apply_ops(&mut self, ops: &[(String, SessionOp)]) -> anyhow::Result<Option<End>> {
        let mut ended = None;
        let mut spawned = 0usize;
        for (tag, op) in ops {
            match op {
                SessionOp::Quit => ended = ended.or(Some(End::Quit)),
                SessionOp::Detach => ended = ended.or(Some(End::Detach)),
                SessionOp::PaneSplit { dir } if spawned >= MAX_NEW_SHELLS_PER_FRAME => {
                    logging::log(
                        Level::Error,
                        tag,
                        &format!(
                            "pane.split('{dir}') refused: at most {MAX_NEW_SHELLS_PER_FRAME} new \
                             shells per frame"
                        ),
                    )
                }
                SessionOp::PaneSplit { dir } => {
                    spawned += 1;
                    self.apply_split(tag, *dir)?;
                }
                SessionOp::PaneFocus { pane } => self.apply_focus(tag, *pane),
                SessionOp::PaneResize { dir, cells } => self.apply_resize(tag, *dir, *cells)?,
                SessionOp::WindowCreate { .. } if spawned >= MAX_NEW_SHELLS_PER_FRAME => {
                    logging::log(
                        Level::Error,
                        tag,
                        &format!(
                            "window.create() refused: at most {MAX_NEW_SHELLS_PER_FRAME} new \
                             shells per frame"
                        ),
                    )
                }
                SessionOp::WindowCreate { window } => {
                    spawned += 1;
                    self.apply_window_create(tag, *window)?;
                }
                SessionOp::WindowFocus { window } => self.apply_window_focus(tag, *window)?,
            }
        }
        Ok(ended)
    }

    fn apply_split(&mut self, tag: &str, dir: neotmux_proto::SplitDir) -> anyhow::Result<()> {
        let new = self.windows.take_pane_id();
        // Read now, before `split`: `split` itself hands focus to `new` the moment the
        // tree allows, and a pane with no shell to read would be answered for by the
        // fallback, hiding the rule it is trying to say.
        let start_dir = self.start_dir(self.windows.current().focus());
        if let Err(refused) = self.windows.current_mut().split(dir, new) {
            logging::log(
                Level::Error,
                tag,
                &format!("pane.split('{dir}') refused: {refused}"),
            );
            return Ok(());
        }

        let started = pane_size(self.windows.rect(new), new).and_then(|size| {
            let mut cmd = PtyHandle::default_shell_command();
            if let Some(dir) = start_dir.clone() {
                cmd.cwd(dir);
            }
            self.shells.open(new, cmd, size)
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
        self.republish_geometry();
        self.steps
            .push(Step::Event(Event::PaneOpened { pane: new }));
        self.steps
            .push(Step::Event(Event::FocusChanged { pane: new }));
        self.dirty = true;
        Ok(())
    }

    /// A new window, with one pane and one shell in it. It is not shown: `window.focus`
    /// is a separate op, so `focus(create())` in one tick opens it and switches to it in that
    /// order while `create()` alone leaves it running out of sight.
    fn apply_window_create(&mut self, tag: &str, window: WindowId) -> anyhow::Result<()> {
        let first = self.windows.create(window, self.surface);
        let start_dir = self.start_dir(self.windows.current().focus());
        let started = pane_size(self.windows.rect(first), first).and_then(|size| {
            let mut cmd = PtyHandle::default_shell_command();
            if let Some(dir) = start_dir {
                cmd.cwd(dir);
            }
            self.shells.open(first, cmd, size)
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
        self.republish_geometry();
        self.steps.push(Step::Event(Event::WindowOpened { window }));
        self.dirty = true;
        Ok(())
    }

    /// Shows another window: its panes are caught up with a surface that may have changed
    /// while they were hidden, then resized to it.
    fn apply_window_focus(&mut self, tag: &str, window: WindowId) -> anyhow::Result<()> {
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
        self.republish_geometry();
        self.steps
            .push(Step::Event(Event::WindowChanged { window }));
        if let Some(focus) = self.windows.current().focus() {
            self.steps
                .push(Step::Event(Event::FocusChanged { pane: focus }));
        }
        self.dirty = true;
        Ok(())
    }

    /// A refusal is logged against whoever asked, as a refused split is: the op is applied
    /// after the tick, so there is nobody left to answer.
    fn apply_resize(
        &mut self,
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
        self.republish_geometry();
        self.dirty = true;
        Ok(())
    }

    fn apply_focus(&mut self, tag: &str, pane: PaneId) {
        if !self.windows.current_mut().set_focus(pane) {
            logging::log(
                Level::Error,
                tag,
                &format!("pane.focus({pane}) refused: no such pane"),
            );
            return;
        }
        self.republish_geometry();
        self.steps.push(Step::Event(Event::FocusChanged { pane }));
        self.dirty = true;
    }

    /// Samples what each pane is running, if the cadence says it is time, and hands the
    /// result to Lua. Reading `/proc` for every pane on every frame is the syscall storm this
    /// exists to avoid, so most frames do nothing here.
    ///
    /// Every pane the session holds, hidden windows included: what a pane in the window you
    /// are not looking at is running is exactly the question a sidebar exists to answer, and
    /// the cadence rather than the number of panes is what bounds the bill.
    fn refresh_pane_info(&mut self) -> Option<BTreeMap<PaneId, PaneInfo>> {
        let shells = &self.shells;
        let panes = self.windows.all_panes();
        let refreshed = self.pane_info.refresh(Instant::now(), panes, |pane| {
            match shells.foreground(pane) {
                Some(pid) => procinfo::read(pid),
                None => Default::default(),
            }
        });
        refreshed.then(|| self.pane_info.snapshot())
    }

    /// Where a new pane's shell begins, and why the answer must always be one. Portable-pty
    /// does not inherit a working directory: an unset one becomes the user's home, so a
    /// caller that leaves it unset is not saying "as you like", it is saying `$HOME`, and
    /// that is the bug this exists to stop. The focused pane's own directory leads — the
    /// foreground process, which is the shell while it is idle and the command it is
    /// running otherwise — and the server's, which is where the client that started it
    /// started it, is the answer when the pane does not exist (the first pane) or when its
    /// directory no longer is one. That last case still arrives as `Some`: a pane that
    /// left its directory behind answers `"<path> (deleted)"`, and it is the `is_dir` check
    /// below — not `None` from `procinfo::cwd` — that filters it out. Remove it and a
    /// deleted directory becomes a pane's starting point again, which is the whole reason
    /// the rule is written down in two places now.
    fn start_dir(&self, pane: Option<PaneId>) -> Option<PathBuf> {
        if let Some(dir) = pane
            .and_then(|pane| self.shells.foreground(pane))
            .and_then(procinfo::cwd)
            .filter(|dir| std::path::Path::new(dir).is_dir())
        {
            return Some(dir.into());
        }
        std::env::current_dir().ok()
    }

    /// Every pane's read stamp. Cheap to gather — it is a number per pane — and it travels
    /// with the tick so that the cheap half of reading a pane stays cheap on the other side
    /// of a socket.
    fn revisions(&self) -> BTreeMap<PaneId, u64> {
        self.windows
            .all_panes()
            .filter_map(|pane| Some((pane, self.terminals.revision(pane)?)))
            .collect()
    }

    /// Puts this session's geometry in the record the client replays, so the very first key
    /// is dispatched against a session that exists.
    fn publish_geometry(&mut self) {
        let geometry = self.geometry();
        self.steps.push(Step::Geometry(geometry));
    }

    /// The tree changed mid-frame. A `pane_opened` handler calling `pane.list()` must see the
    /// pane it is being told about, and a `pane_closed` handler must not see the dead one —
    /// so the geometry catches up before any of those events is recorded.
    fn republish_geometry(&mut self) {
        self.publish_geometry();
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

    /// The shared part of any picture: the session's dirty and the panes' grids are
    /// frozen before the render, because a render is what clears the dirty, and a hidden
    /// pane's terminal goes on taking its child's output, but copying its grid into a
    /// frame nobody draws it in would bill every window onto every frame.
    fn snapshot(&mut self) {
        let shown: Vec<PaneId> = self.windows.current().rects.keys().copied().collect();
        self.shells.snapshot(shown.into_iter());
        self.dirty = false;
    }

    /// The picture itself, and nothing else: the panes and the rects are the session's,
    /// as everything else about it is; the overlay and the notices are the caller's,
    /// passed in because they are this client's and the last ones sent are not necessarily
    /// this client's.
    fn picture<'a>(&'a self, overlay: &'a [DrawCmd], notices: Vec<Notice>) -> Frame<'a> {
        let layout = self.windows.current();
        Frame {
            size: self.surface.size,
            panes: layout
                .showable()
                .filter_map(|(pane, rect)| {
                    Some(PaneFrame {
                        pane,
                        rect,
                        grid: self.shells.grid(pane)?,
                    })
                })
                .collect(),
            rects: &layout.rects,
            cursor: layout.focus(),
            overlay,
            notices,
        }
    }

    /// The session's picture with a client's overlay and notices on it: the caller brings
    /// the overlay and the notices, because they are that client's, and the caller keeps
    /// them after the frame, which is the whole of why the frame may borrow them.
    pub fn frame_with<'a>(&'a mut self, overlay: &'a [DrawCmd], notices: Vec<Notice>) -> Frame<'a> {
        self.snapshot();
        self.picture(overlay, notices)
    }

    /// What applying a tock did to the session: the shared part, ops and geometry and
    /// the end, in the order they were asked. Not the picture — the picture is the
    /// caller's to make of this, with the overlay and notices the caller knows to be
    /// this client's own, and this is the whole of why a tock with two clients is a
    /// tock with one.
    pub fn apply(&mut self, tock: &Tock) -> anyhow::Result<Apply> {
        if let Some(end) = self.apply_ops(&tock.ops)? {
            return Ok(Apply {
                steps: std::mem::take(&mut self.steps),
                end: Some(end),
            });
        }
        let reserved_changed = self.reserved_changed(tock.reserved);
        let gutter_changed = self.gutter_changed(tock.gutter);
        if reserved_changed || gutter_changed {
            self.apply_geometry()?;
        }
        let eof = self.ended;
        Ok(Apply {
            steps: std::mem::take(&mut self.steps),
            end: eof.then_some(End::Eof),
        })
    }

    /// Whether the session changed in the way a frame is for, whatever the client's own
    /// overlay is about to add on top. A client decides whether to force one; the
    /// session only answers this half.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The one subscription that is the session's and not a client's: the pipe is the
    /// session's, and a client asking for it is a client asking for the session to keep
    /// reading it. The server holds the OR of who is asking, and this is how it says so.
    pub fn wants_output(&mut self, wanted: bool) {
        self.wants_pane_output = wanted;
    }

    /// Keeps the session running with nobody looking at it: the children go on being read,
    /// so one that fills its pipe is not blocked by a client that closed its terminal.
    /// `true` means the last of them is gone and the session is over.
    ///
    /// Whatever happened is kept rather than thrown away, so whoever looks next is told.
    pub fn idle(&mut self) -> anyhow::Result<bool> {
        if self.drain_panes()? == Drained::Eof {
            self.ended = true;
        }
        Ok(self.ended)
    }

    /// Nobody is subscribed to anything any more, because nobody is running any plugins.
    /// Without this an unattached session would go on copying every pane's output into a
    /// record no client has asked for.
    pub fn forget_pane_output(&mut self) {
        self.wants_output(false);
        self.steps
            .retain(|step| !matches!(step, Step::Event(Event::PaneOutput { .. })));
    }

    /// The first half of a frame: the panes' output, and everything the client's Lua layer
    /// has to be told before it ticks. The tick itself happens in the other process.
    pub fn tick(&mut self) -> anyhow::Result<Tick> {
        if self.drain_panes()? == Drained::Eof {
            self.ended = true;
        }
        let pane_info = self.refresh_pane_info();
        Ok(Tick {
            steps: std::mem::take(&mut self.steps),
            epoch: self.windows.current().epoch(),
            geometry: self.geometry(),
            pane_info,
            revisions: self.revisions(),
        })
    }

    /// The second half: what the tick asked for, applied, and the frame that came of it.
    /// The frame is `None` when nothing changed and the last one still stands.
    pub fn advance(&mut self, tock: &Tock) -> anyhow::Result<Pass<'_>> {
        self.wants_output(tock.wants_pane_output);
        let application = self.apply(tock)?;
        // The picture this tock asked for is committed before the end is looked at, on
        // every path but an ops-Quit (which the original also skipped it on), so the
        // last frame a leaving session shows is this tock's one, and not the one before.
        if application.end != Some(End::Quit) {
            if tock.overlay != self.last_overlay {
                self.dirty = true;
            }
            self.last_overlay.clone_from(&tock.overlay);
            if tock.notices != self.last_notices {
                self.dirty = true;
            }
            self.last_notices.clone_from(&tock.notices);
        }
        match application.end {
            // The original forced this last frame (`if self.dirty || eof`); folding the
            // `Eof` into `apply`'s `end` must keep the frame too, not only the end.
            Some(End::Eof) => {
                self.snapshot();
                let frame = Some(self.picture(&self.last_overlay, tock.notices.clone()));
                Ok(Pass {
                    steps: application.steps,
                    frame,
                    end: Some(End::Eof),
                })
            }
            Some(end) => Ok(Pass {
                steps: application.steps,
                frame: None,
                end: Some(end),
            }),
            None => {
                let frame = if self.dirty {
                    self.snapshot();
                    Some(self.picture(&self.last_overlay, tock.notices.clone()))
                } else {
                    None
                };
                Ok(Pass {
                    steps: application.steps,
                    frame,
                    end: None,
                })
            }
        }
    }
}

/// What one pass of the session produced.
pub struct Pass<'a> {
    /// What applying the tick's reply did to the session, in order. It is answered in the
    /// same breath as the frame rather than held for the next tick: a key handler must see
    /// the session as it is, and between the reply and this it is not.
    pub steps: Vec<Step>,
    /// The frame to show, if this pass changed anything.
    pub frame: Option<Frame<'a>>,
    /// Why the session is over, if it is.
    pub end: Option<End>,
}

/// What one tock did to a session, without the picture it is shown in.
pub struct Apply {
    /// What applying the tock's ops did to the session, in order, for the reason `Pass`
    /// already gives it.
    pub steps: Vec<Step>,
    /// Why the session is over, if it is: a `Quit` a client's op raised, or an `Eof` the
    /// last of its shells just went.
    pub end: Option<End>,
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

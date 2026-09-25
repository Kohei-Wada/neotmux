//! The half of a frame that belongs to the plugins.
//!
//! The Lua layer runs beside the panes, in the server, because the most interesting thing a
//! plugin can do — watch a long-running job while nobody is looking at it — is a thing it can
//! only do while it is running. What that costs is a plugin that freezes taking the session
//! with it, which is what the budget in `neotmux-lua` is for.
//!
//! What is here is that half on its own — a [`Tick`] in, a [`Tock`] out, and not one line
//! that names a terminal. The session is on the other side of it, and it is the next function
//! call rather than a socket.

use crate::Answer;
use crate::Notifications;
use crate::ServerOp;
use crate::input::{encode_key, key_spec};
use neotmux_lua::host::Op;
use neotmux_lua::{KeyOutcome, LuaSubsystem, WriteVerdict};
use neotmux_proto::wire::{SessionOp, Step, Tick, Tock};
use neotmux_proto::{Key, Level, PaneId, PaneTty, SessionId};
use neotmux_sys::logging;
use std::collections::BTreeMap;
use std::time::Instant;

/// The Lua subsystem and the notification surface that goes with it, driven a frame at a
/// time.
pub struct Plugins {
    lua: LuaSubsystem,
    notifications: Notifications,
}

impl Plugins {
    pub fn new(lua: LuaSubsystem) -> Self {
        Self {
            lua,
            notifications: Notifications::new(),
        }
    }

    pub fn lua(&self) -> &LuaSubsystem {
        &self.lua
    }

    /// Everything raised this session, for replaying on stderr once the terminal is back.
    pub fn notifications(&self) -> &Notifications {
        &self.notifications
    }

    /// The one place a write is checked, and the one place a refusal is announced.
    ///
    /// With nothing listening this is a registry lookup and no more, which is what keeps a
    /// server whose layers register no handler on exactly the path it has today.
    pub fn check_write(
        &mut self,
        session: SessionId,
        pane: PaneId,
        text: &str,
        from: &str,
    ) -> Result<(), String> {
        if !self.lua.has_write_hooks() {
            return Ok(());
        }
        match self.lua.dispatch_write(session, pane, text, from) {
            WriteVerdict::Pass => Ok(()),
            WriteVerdict::Refused(why) => {
                let said = format!("write to {pane} refused: {why}");
                logging::log(Level::Warn, "neotmux", &said);
                self.notifications.push(Level::Warn, "neotmux", &said);
                Err(why)
            }
        }
    }

    /// Offers a key to the plugins. `Fallthrough` means nobody wanted it, and it belongs to
    /// whichever pane has the focus — which is the session's to know, not this side's. The
    /// flag is whether taking it raised an op, which only the next frame carries out.
    pub fn key(&self, session: SessionId, key: Key) -> (KeyOutcome, bool) {
        let before = self.lua.ops_raised();
        let outcome = self.lua.dispatch_key(session, &key_spec(&key));
        (outcome, self.lua.ops_raised() > before)
    }

    /// What the session did, replayed into the Lua layer in the order it happened.
    pub fn catch_up(&self, session: SessionId, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Geometry(geometry) => self.lua.republish_geometry(session, geometry),
                Step::Bands { surface, applied } => {
                    self.lua.lay_out_reservations(session, surface, applied)
                }
                Step::Event(event) => self.lua.dispatch_event(session, &event),
            }
        }
    }

    /// One frame's worth of plugin work: catch the Lua layer up with what the session did,
    /// tick it, and answer with what it asked for and what it drew.
    pub fn respond(&mut self, session: SessionId, tick: Tick) -> Answer {
        self.catch_up(session, tick.steps);
        if let Some(info) = tick.pane_info {
            self.lua.publish_pane_info(session, info);
        }
        // Kept as well as published: converting a relayed key needs to know whether the pane
        // it is going to asked for application cursor keys, and that is the same fact.
        let tty = tick.pane_tty.clone();
        self.lua.publish_pane_tty(session, tick.pane_tty);
        self.lua.begin_frame(session, tick.epoch, tick.geometry);
        // Before the tick, so a plugin drawing the surface draws what is on it now.
        self.notifications.expire(Instant::now());
        self.lua.publish_notices(self.notifications.visible());
        // Before the tick, so a tick sees what this frame's jobs and timers have already
        // changed rather than finding out next frame.
        self.lua.dispatch_jobs(session);
        self.lua.dispatch_timers(session, Instant::now());
        self.lua.dispatch_tick(session);

        let (ops, server) = self.drain_ops(Some(session), &tty);
        let tock = Tock {
            ops,
            overlay: self.lua.frame_output(),
            reserved: self.lua.reserved(),
            gutter: self.lua.opt().pane_gutter,
            wants_pane_output: self.lua.has_subscriber("pane_output"),
        };
        Answer {
            tock,
            server,
            views: self.lua.frame_views(),
        }
    }

    /// The ops the session has to apply, with the name of whoever raised each. `log` and
    /// `notify` never leave: the source that raised them and the transcript that replays
    /// them are both here.
    #[allow(clippy::type_complexity)]
    fn drain_ops(
        &mut self,
        session: Option<SessionId>,
        tty: &BTreeMap<PaneId, PaneTty>,
    ) -> (Vec<(String, SessionOp)>, Vec<(String, ServerOp)>) {
        let mut out = Vec::new();
        let mut server = Vec::new();
        // `drain_ops` hands back an owned Vec, so no borrow of `self.lua` survives into this
        // loop — a `PaneSend` is checked right here, at the position it sits, rather than
        // moved to a second pass that would reorder it past whatever came after it.
        for (id, op) in self.lua.drain_ops(session) {
            let tag = self
                .lua
                .sources()
                .name(id)
                .unwrap_or_else(|| id.to_string());
            let op = match op {
                Op::Log { level, text } => {
                    logging::log(level, &tag, &text);
                    continue;
                }
                Op::Notify { level, text } => {
                    self.notifications.push(level, &tag, &text);
                    logging::log(level, &tag, &text);
                    continue;
                }
                Op::NoticesClear => {
                    self.notifications.clear();
                    continue;
                }
                Op::Quit => SessionOp::Quit,
                Op::Detach => SessionOp::Detach,
                Op::PaneSplit { dir } => SessionOp::PaneSplit { dir },
                Op::PaneFocus { pane } => SessionOp::PaneFocus { pane },
                Op::PaneResize { dir, cells } => SessionOp::PaneResize { dir, cells },
                Op::PaneSend { pane, text } => {
                    // After the tick has returned, so the VM is idle: a handler is never
                    // re-entered from inside the tick that queued this. With no session yet
                    // (a load op), there is nothing to check against and no pane for it to
                    // land in either, so it passes through unchecked.
                    if let Some(session) = session
                        && self.check_write(session, pane, &text, "plugin").is_err()
                    {
                        continue;
                    }
                    SessionOp::PaneSend { pane, text }
                }
                Op::PaneKey { pane, spec } => {
                    // The same path a `pane.send` takes, once the keystroke is the bytes the
                    // pane would have been sent had somebody pressed it there: a write hook
                    // sees a relayed key like any other write, and the pane's own DECCKM is
                    // what decides how a cursor key is spelled.
                    let Some(key) = Key::from_spec(&spec) else {
                        logging::log(Level::Error, &tag, &format!("pane.key: no key is '{spec}'"));
                        continue;
                    };
                    let application = tty.get(&pane).is_some_and(|tty| tty.application_cursor);
                    let Some(bytes) = encode_key(key, application) else {
                        continue;
                    };
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if let Some(session) = session
                        && self.check_write(session, pane, &text, "plugin").is_err()
                    {
                        continue;
                    }
                    SessionOp::PaneSend { pane, text }
                }
                Op::WindowCreate { window } => SessionOp::WindowCreate { window },
                Op::WindowFocus { window } => SessionOp::WindowFocus { window },
                Op::WindowRename { window, name } => SessionOp::WindowRename { window, name },
                Op::SessionCreate { name } => {
                    server.push((tag, ServerOp::Create { name }));
                    continue;
                }
                Op::SessionSwitch { name } => {
                    server.push((tag, ServerOp::Switch { name }));
                    continue;
                }
                Op::SessionKill { name } => {
                    server.push((tag, ServerOp::Kill { name }));
                    continue;
                }
                Op::SessionRename { from, to } => {
                    server.push((tag, ServerOp::Rename { from, to }));
                    continue;
                }
                Op::Reload => {
                    server.push((tag, ServerOp::Reload));
                    continue;
                }
            };
            out.push((tag, op));
        }
        (out, server)
    }

    /// The ops a session's teardown raised, once that session is gone. A job's `on_exit` runs
    /// inside `forget_session`, with the dying session still bound, so what it raises is filed
    /// under a session that will never have another frame: nobody else would ever come for
    /// these, and the buffer would hold them for the life of the server.
    ///
    /// Only the three that need nothing but this side survive the trip. Everything else wants
    /// a live session to act on and has nowhere left to land — `quit` and `detach`, every
    /// `pane.*` and `window.*`, and the `session.*` four — so it is dropped here rather than
    /// deferred to a frame that is not coming.
    pub fn drain_teardown_ops(&mut self, session: SessionId) {
        for (id, op) in self.lua.drain_ops(Some(session)) {
            let tag = self
                .lua
                .sources()
                .name(id)
                .unwrap_or_else(|| id.to_string());
            match op {
                Op::Log { level, text } => logging::log(level, &tag, &text),
                Op::Notify { level, text } => {
                    self.notifications.push(level, &tag, &text);
                    logging::log(level, &tag, &text);
                }
                Op::NoticesClear => self.notifications.clear(),
                _ => {}
            }
        }
    }

    /// The ops a layer raised while it was still loading, before there was a session for any
    /// of them to act on. Returns whether one of them asked to quit, which is the one that
    /// can be honoured: nothing is running yet, so there is nothing to split or focus.
    pub fn drain_load_ops(&mut self) -> bool {
        let mut quit = false;
        // The load ran before any session existed, so what it raised belongs to none of them —
        // and no pane has said anything about how it wants its keys, because no pane exists.
        let (ops, server) = self.drain_ops(None, &BTreeMap::new());
        for (tag, op) in server {
            let (call, why) = match op {
                ServerOp::Create { name } => (
                    format!("session.create('{name}')"),
                    "no session exists until one starts",
                ),
                ServerOp::Switch { name } => (
                    format!("session.switch('{name}')"),
                    "there is nothing to switch away from until a session starts",
                ),
                ServerOp::Kill { name } => (
                    format!("session.kill('{name}')"),
                    "no session exists until one starts",
                ),
                ServerOp::Rename { from, to } => (
                    format!("session.rename('{from}', '{to}')"),
                    "no session exists until one starts",
                ),
                ServerOp::Reload => (
                    "neotmux.reload()".to_string(),
                    "the config is loading already",
                ),
            };
            refuse(&tag, &call, why);
        }
        for (tag, op) in ops {
            match op {
                SessionOp::Quit => quit = true,
                SessionOp::Detach => refuse(
                    &tag,
                    "neotmux.detach()",
                    "there is no session to detach from until it starts",
                ),
                SessionOp::PaneSplit { dir } => refuse(
                    &tag,
                    &format!("pane.split('{dir}')"),
                    "no panes exist until the session starts",
                ),
                SessionOp::PaneResize { dir, cells } => refuse(
                    &tag,
                    &format!("pane.resize('{dir}', {cells})"),
                    "no panes exist until the session starts",
                ),
                SessionOp::PaneFocus { pane } => refuse(
                    &tag,
                    &format!("pane.focus({pane})"),
                    "no panes exist until the session starts",
                ),
                SessionOp::PaneSend { pane, .. } => refuse(
                    &tag,
                    &format!("pane.send({pane}, ...)"),
                    "no panes exist until the session starts",
                ),
                SessionOp::WindowCreate { window } => refuse(
                    &tag,
                    "window.create()",
                    &format!("{window} cannot be opened until the session starts"),
                ),
                SessionOp::WindowFocus { window } => refuse(
                    &tag,
                    &format!("window.focus({window})"),
                    "no windows exist until the session starts",
                ),
                SessionOp::WindowRename { window, .. } => refuse(
                    &tag,
                    &format!("window.rename({window}, ...)"),
                    "no windows exist until the session starts",
                ),
            }
        }
        quit
    }

    /// Runs a reload the server carried out of the frame that asked for it, once every other
    /// op of that frame was already applied. Every failure the reload raised is logged at
    /// warn, and whatever the reloaded layers raised while loading is refused: a load acts on
    /// no session, so nothing it asked for has anywhere to land.
    pub fn reload(&mut self, session: Option<SessionId>) {
        for failure in self.lua.reload(session) {
            logging::log(Level::Warn, &failure.source, &failure.message);
        }
        let (ops, server) = self.drain_ops(None, &BTreeMap::new());
        let why = "the config was reloading, and a load acts on no session";
        for (tag, op) in server {
            refuse(&tag, &format!("{op:?}"), why);
        }
        for (tag, op) in ops {
            refuse(&tag, &format!("{op:?}"), why);
        }
    }
}

fn refuse(tag: &str, call: &str, why: &str) {
    logging::log(Level::Error, tag, &format!("{call} refused: {why}"));
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use neotmux_lua::LuaSubsystem;
    use neotmux_lua::host::{JobChunk, JobSpec, Runner, Running};
    use neotmux_lua::plugin::Layers;
    use neotmux_proto::geom::Window;
    use neotmux_proto::{Epoch, Geometry, PaneId, Rect, SessionId, Size, WindowId};
    use neotmux_pty::PaneTerminals;

    use super::*;

    fn built(script: &str) -> (tempfile::TempDir, Plugins) {
        let dir = tempfile::tempdir().expect("layer dir");
        std::fs::write(dir.path().join("init.lua"), script).expect("write layer file");
        let terminals = Rc::new(PaneTerminals::new());
        let lua = LuaSubsystem::build(
            Layers::from_paths(vec![dir.path().to_path_buf()]),
            terminals,
        )
        .expect("lua");
        assert!(lua.run_init().is_empty());
        (dir, Plugins::new(lua))
    }

    /// A job that says nothing and needs no reaping. Enough for one to be live when the
    /// session ends, which is the only thing that calls `on_exit` from inside
    /// `forget_session`.
    struct Idle;

    impl Running for Idle {
        fn poll(&mut self) -> Vec<JobChunk> {
            Vec::new()
        }

        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Fake;

    impl Runner for Fake {
        fn start(&self, _spec: &JobSpec) -> std::io::Result<Box<dyn Running>> {
            Ok(Box::new(Idle))
        }
    }

    fn built_with_jobs(script: &str) -> (tempfile::TempDir, Plugins) {
        let dir = tempfile::tempdir().expect("layer dir");
        std::fs::write(dir.path().join("init.lua"), script).expect("write layer file");
        let lua = LuaSubsystem::build_with_runner(
            Layers::from_paths(vec![dir.path().to_path_buf()]),
            Rc::new(PaneTerminals::new()),
            Rc::new(neotmux_lua::host::Absent),
            Rc::new(Fake),
        )
        .expect("lua");
        assert!(lua.run_init().is_empty());
        (dir, Plugins::new(lua))
    }

    fn begin(plugins: &Plugins, session: SessionId) {
        let pane = PaneId::from(1);
        let size = Size { rows: 24, cols: 80 };
        plugins.lua.begin_frame(
            session,
            Epoch::new(size, vec![pane], Some(pane)),
            Geometry {
                area: Rect::new(0, 0, size.cols, size.rows),
                panes: vec![(pane, Rect::new(0, 0, size.cols, size.rows))],
                focus: Some(pane),
                ..Default::default()
            },
        );
    }

    /// A job's `on_exit` runs inside `forget_session`, with the dying session bound, so what
    /// it raises is filed under a session no frame will ever ask for again. Without the
    /// teardown drain the first thing an author writes to debug a teardown reaches neither
    /// the log nor the transcript, and the buffer keeps it for the life of the server.
    #[test]
    fn what_a_teardown_on_exit_says_still_arrives() {
        let (_dir, mut plugins) = built_with_jobs(
            "local started = false
             neotmux.on_tick(function()
               if started then return end
               started = true
               neotmux.job.start({ 'sleep' }, {
                 on_exit = function()
                   neotmux.log.warn('teardown said so')
                   neotmux.notify('warn', 'and so')
                 end,
               })
             end)",
        );
        let session = SessionId::from(1);
        begin(&plugins, session);
        plugins.lua.dispatch_tick(session);
        plugins.drain_ops(Some(session), &BTreeMap::new());

        plugins.lua.forget_session(session);
        plugins.drain_teardown_ops(session);

        assert_eq!(
            plugins.notifications().since(0),
            vec!["init: and so".to_string()],
            "a teardown's notify never reached the transcript"
        );
        assert!(
            plugins.lua.drain_ops(Some(session)).is_empty(),
            "a teardown's ops were left in the buffer for nobody"
        );
    }

    /// `Op::PaneSend` used to be converted in a second pass, after every other op in the
    /// tick — a plugin doing split, then send, then focus in one tick would see its send
    /// arrive last. The conversion has to happen where the op sits, or op order (which is
    /// load-bearing for a session) is not what a plugin raised.
    #[test]
    fn a_pane_send_is_converted_in_the_order_it_was_raised() {
        let (_dir, mut plugins) = built(
            "neotmux.on_tick(function()
               neotmux.pane.split('horizontal')
               neotmux.pane.send(1, 'hi')
               neotmux.pane.focus(1)
             end)",
        );
        let session = SessionId::from(1);
        let pane = PaneId::from(1);
        let size = Size { rows: 24, cols: 80 };
        plugins.lua.begin_frame(
            session,
            Epoch::new(size, vec![pane], Some(pane)),
            Geometry {
                area: Rect::new(0, 0, size.cols, size.rows),
                windows: vec![Window {
                    id: WindowId::from(1),
                    panes: vec![pane],
                    name: None,
                }],
                window: Some(WindowId::from(1)),
                panes: vec![(pane, Rect::new(0, 0, size.cols, size.rows))],
                focus: Some(pane),
            },
        );
        plugins.lua.dispatch_tick(session);
        let (ops, _server) = plugins.drain_ops(Some(session), &BTreeMap::new());

        let kinds: Vec<&str> = ops
            .iter()
            .map(|(_, op)| match op {
                SessionOp::PaneSplit { .. } => "split",
                SessionOp::PaneSend { .. } => "send",
                SessionOp::PaneFocus { .. } => "focus",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["split", "send", "focus"], "{ops:?}");
    }

    #[test]
    fn a_reload_raised_by_a_key_reaches_the_server_as_a_server_op() {
        let (_dir, mut plugins) =
            built("neotmux.keymap.set('C-r', function() neotmux.reload() return true end)");
        let session = SessionId::from(1);
        begin(&plugins, session);
        plugins.key(session, Key::from_spec("C-r").unwrap());
        let (_ops, server) = plugins.drain_ops(Some(session), &BTreeMap::new());
        assert!(
            server.iter().any(|(_, op)| matches!(op, ServerOp::Reload)),
            "{server:?}"
        );
    }
}

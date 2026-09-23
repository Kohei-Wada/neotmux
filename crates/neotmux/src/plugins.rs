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

use std::collections::BTreeMap;

use neotmux_lua::ops::Op;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::msg::{SessionOp, Step, Tick, Tock};
use neotmux_proto::{Key, Level, PaneId, SessionId};
use std::time::Instant;

use crate::input::key_spec;
use crate::logging;
use crate::notify::Notifications;

/// What a plugin asked of the *server* rather than of a session. The set of sessions is not
/// something a session can change, so these do not travel in a tock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerOp {
    Create { name: String },
    Switch { name: String },
    Kill { name: String },
    Rename { from: String, to: String },
}

/// One frame's worth of answer from the plugins: what the session has to apply, what the
/// *server* has to apply, and which panes this frame shows at a place in their own history.
pub struct Answer {
    pub tock: Tock,
    pub server: Vec<(String, ServerOp)>,
    pub views: BTreeMap<PaneId, usize>,
}

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

    /// Offers a key to the plugins. `Fallthrough` means nobody wanted it, and it belongs to
    /// whichever pane has the focus — which is the session's to know, not this side's.
    pub fn key(&self, session: SessionId, key: Key) -> KeyOutcome {
        self.lua.dispatch_key(session, &key_spec(&key))
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
        self.lua.publish_pane_tty(session, tick.pane_tty);
        self.lua.begin_frame(session, tick.epoch, tick.geometry);
        // Before the tick, so a plugin drawing the surface draws what is on it now.
        self.notifications.expire(Instant::now());
        self.lua.publish_notices(self.notifications.visible());
        self.lua.dispatch_tick(session);

        let (ops, server) = self.drain_ops(Some(session));
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
    ) -> (Vec<(String, SessionOp)>, Vec<(String, ServerOp)>) {
        let mut out = Vec::new();
        let mut server = Vec::new();
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
                Op::PaneSend { pane, text } => SessionOp::PaneSend { pane, text },
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
            };
            out.push((tag, op));
        }
        (out, server)
    }

    /// The ops a layer raised while it was still loading, before there was a session for any
    /// of them to act on. Returns whether one of them asked to quit, which is the one that
    /// can be honoured: nothing is running yet, so there is nothing to split or focus.
    pub fn drain_load_ops(&mut self) -> bool {
        let mut quit = false;
        // The load ran before any session existed, so what it raised belongs to none of them.
        let (ops, server) = self.drain_ops(None);
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
}

fn refuse(tag: &str, call: &str, why: &str) {
    logging::log(Level::Error, tag, &format!("{call} refused: {why}"));
}

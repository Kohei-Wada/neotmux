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

use neotmux_lua::ops::Op;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::msg::{SessionOp, Step, Tick, Tock};
use neotmux_proto::{Key, Level};
use std::time::Instant;

use crate::input::key_spec;
use crate::logging;
use crate::notify::Notifications;

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
    pub fn key(&self, key: Key) -> KeyOutcome {
        self.lua.dispatch_key(&key_spec(&key))
    }

    /// What the session did, replayed into the Lua layer in the order it happened.
    pub fn catch_up(&self, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Geometry(geometry) => self.lua.republish_geometry(geometry),
                Step::Bands { surface, applied } => self.lua.lay_out_reservations(surface, applied),
                Step::Event(event) => self.lua.dispatch_event(&event),
            }
        }
    }

    /// One frame's worth of plugin work: catch the Lua layer up with what the session did,
    /// tick it, and answer with what it asked for and what it drew.
    pub fn respond(&mut self, tick: Tick) -> Tock {
        self.catch_up(tick.steps);
        if let Some(info) = tick.pane_info {
            self.lua.publish_pane_info(info);
        }
        self.lua.begin_frame(tick.epoch, tick.geometry);
        self.lua.dispatch_tick();

        let ops = self.drain_ops();
        self.notifications.expire(Instant::now());
        Tock {
            ops,
            overlay: self.lua.frame_output(),
            notices: self.notifications.visible(),
            reserved: self.lua.reserved(),
            gutter: self.lua.opt().pane_gutter,
            wants_pane_output: self.lua.has_subscriber("pane_output"),
        }
    }

    /// The ops the session has to apply, with the name of whoever raised each. `log` and
    /// `notify` never leave: the source that raised them and the transcript that replays
    /// them are both here.
    fn drain_ops(&mut self) -> Vec<(String, SessionOp)> {
        let mut out = Vec::new();
        for (id, op) in self.lua.drain_ops() {
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
                Op::Quit => SessionOp::Quit,
                Op::Detach => SessionOp::Detach,
                Op::PaneSplit { dir } => SessionOp::PaneSplit { dir },
                Op::PaneFocus { pane } => SessionOp::PaneFocus { pane },
                Op::PaneResize { dir, cells } => SessionOp::PaneResize { dir, cells },
                Op::WindowCreate { window } => SessionOp::WindowCreate { window },
                Op::WindowFocus { window } => SessionOp::WindowFocus { window },
            };
            out.push((tag, op));
        }
        out
    }

    /// The ops a layer raised while it was still loading, before there was a session for any
    /// of them to act on. Returns whether one of them asked to quit, which is the one that
    /// can be honoured: nothing is running yet, so there is nothing to split or focus.
    pub fn drain_load_ops(&mut self) -> bool {
        let mut quit = false;
        for (tag, op) in self.drain_ops() {
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
            }
        }
        quit
    }
}

fn refuse(tag: &str, call: &str, why: &str) {
    logging::log(Level::Error, tag, &format!("{call} refused: {why}"));
}

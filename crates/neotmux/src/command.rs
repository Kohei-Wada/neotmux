//! The commands the host owns.
//!
//! These control the server — its sessions, their panes, what is written into one and what is
//! on it — and they are answered without the Lua layer being consulted, entered or woken. That
//! is the whole of why they are here rather than in the runtime layer: a control CLI that
//! stops working over how a config is written is not a control CLI.
//!
//! Nothing in this file knows about the socket. A name, its arguments and the sessions go in;
//! lines and whether it worked come out.

use std::rc::Rc;

use neotmux_proto::Size;
use neotmux_term::terminals::PaneTerminals;
use neotmux_term::wakeup::Waker;

use neotmux::plugins::Plugins;

use crate::server::{RenameFailure, Sessions, StartFailure, rename_session, start_session};

/// What a command did.
pub struct Ran {
    pub ok: bool,
    /// stdout where `ok`, and the one sentence that says why not where it is not.
    pub lines: Vec<String>,
}

impl Ran {
    pub fn ok(lines: Vec<String>) -> Self {
        Self { ok: true, lines }
    }

    /// The one line a command answers with when it has nothing more to say than that it did.
    pub fn did(line: impl Into<String>) -> Self {
        Self {
            ok: true,
            lines: vec![line.into()],
        }
    }

    pub fn failed(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            lines: vec![why.into()],
        }
    }
}

/// Every name the host answers. The CLI reads it to tell a command from a session name.
pub const NAMES: &[&str] = &["new", "rename"];

pub fn unknown(name: &str) -> Ran {
    Ran::failed(format!("unknown command '{name}'"))
}

/// What a command reaches into the server for. Built fresh at the door, out of whatever
/// `Serving` and the frame loop already hold.
pub struct Ctx<'a> {
    pub sessions: &'a mut Sessions,
    pub plugins: &'a Plugins,
    pub terminals: &'a Rc<PaneTerminals>,
    pub waker: &'a Waker,
}

pub fn run(name: &str, args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    match name {
        "new" => new(args, session, ctx),
        "rename" => rename(args, ctx),
        other => unknown(other),
    }
}

/// A session started with no client attached. Its surface is 80x24 because a surface has to
/// be some size before a shell can open on it, and the first client to attach refits it —
/// the `window-size latest` rule sessions already follow.
const HEADLESS: Size = Size { rows: 24, cols: 80 };

fn new(args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let name = args
        .first()
        .map(String::as_str)
        .or(session)
        .unwrap_or(neotmux::sock::DEFAULT_SESSION)
        .to_string();
    match start_session(
        &name,
        HEADLESS,
        ctx.plugins,
        ctx.sessions,
        ctx.terminals,
        ctx.waker,
    ) {
        Ok(()) => Ran::ok(vec![name]),
        Err(StartFailure::Invalid(why)) => Ran::failed(why),
        Err(StartFailure::Taken) => {
            Ran::failed(format!("a session called '{name}' is already running"))
        }
        Err(StartFailure::Failed(err)) => Ran::failed(format!("could not start '{name}': {err:#}")),
    }
}

fn rename(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let [from, to] = args else {
        return Ran::failed("usage: neotmux rename <old> <new>");
    };
    match rename_session(from, to, ctx.plugins, ctx.sessions) {
        Ok(()) => Ran::did(to.clone()),
        Err(RenameFailure::Invalid(why)) => Ran::failed(why),
        Err(RenameFailure::Taken) => {
            Ran::failed(format!("a session called '{to}' is already running"))
        }
        Err(RenameFailure::Missing) => {
            Ran::failed(format!("neotmux session '{from}' is not running"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_nobody_implements_is_a_failure_that_says_so() {
        let ran = unknown("frobnicate");
        assert!(!ran.ok);
        assert_eq!(ran.lines, vec!["unknown command 'frobnicate'".to_string()]);
    }
}

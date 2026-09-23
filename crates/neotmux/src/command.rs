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

use neotmux_proto::{PaneId, Size, Want};
use neotmux_term::terminals::PaneTerminals;
use neotmux_term::wakeup::Waker;

use neotmux::plugins::Plugins;

use neotmux::session::Session;

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
pub const NAMES: &[&str] = &["new", "rename", "windows", "panes", "send", "capture"];

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
        "windows" => windows(session, ctx),
        "panes" => panes(session, ctx),
        "send" => send(args, ctx),
        "capture" => capture(args, ctx),
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

/// The session a command is addressed to, or the sentence that says there is none.
fn addressed<'a>(session: Option<&str>, ctx: &'a mut Ctx<'_>) -> Result<&'a mut Session, Ran> {
    let name = session.unwrap_or(neotmux::sock::DEFAULT_SESSION);
    match ctx.sessions.id_of(name) {
        Some(id) => Ok(ctx.sessions.get_mut(id).expect("a session it just named")),
        None => Err(Ran::failed(format!(
            "neotmux session '{name}' is not running"
        ))),
    }
}

fn windows(session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let held = match addressed(session, ctx) {
        Ok(held) => held,
        Err(no) => return no,
    };
    Ran::ok(
        held.windows_list()
            .into_iter()
            .map(|(id, panes, name)| {
                format!(
                    "@{} {} panes={}",
                    id.get(),
                    name.as_deref().unwrap_or("-"),
                    panes.len()
                )
            })
            .collect(),
    )
}

fn panes(session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let terminals = ctx.terminals.clone();
    let held = match addressed(session, ctx) {
        Ok(held) => held,
        Err(no) => return no,
    };
    Ran::ok(
        held.pane_rows()
            .into_iter()
            .map(|row| {
                let size = terminals
                    .size(row.pane)
                    .map(|size| format!("{}x{}", size.cols, size.rows))
                    .unwrap_or_else(|| "-".to_string());
                format!(
                    "%{} @{} {} {} {}",
                    row.pane.get(),
                    row.window.get(),
                    size,
                    row.command.as_deref().unwrap_or("-"),
                    row.cwd.as_deref().unwrap_or("-"),
                )
            })
            .collect(),
    )
}

/// `%4`, the way a pane is written everywhere else. A bare number is taken too: a caller that
/// has the id out of a `panes` line should not have to remember the sigil.
fn pane_id(word: &str) -> Option<PaneId> {
    word.trim_start_matches('%')
        .parse::<u32>()
        .ok()
        .map(PaneId::from)
}

/// `--enter` is the flag only in the last position; anywhere else it is payload, the same as
/// any other word. `send`'s whole job is putting exactly this text in that pane, so a flag it
/// ate out of the middle of the line would be a payload it silently changed.
fn split_enter(rest: &[String]) -> (String, bool) {
    match rest.split_last() {
        Some((last, body)) if last == "--enter" => (body.join(" "), true),
        _ => (rest.join(" "), false),
    }
}

fn send(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let Some((first, rest)) = args.split_first() else {
        return Ran::failed("usage: neotmux send <pane> <text> [--enter]");
    };
    let Some(pane) = pane_id(first) else {
        return Ran::failed(format!("'{first}' is not a pane id"));
    };
    let (text, enter) = split_enter(rest);
    let mut bytes = text.into_bytes();
    if enter {
        bytes.push(b'\r');
    }
    // Pane ids are the server's, so the pane is looked for in every session rather than in the
    // one `-t` named — which is why `send` takes no `-t` at all.
    for id in ctx.sessions.ids() {
        let Some(session) = ctx.sessions.get_mut(id) else {
            continue;
        };
        match session.write_to_pane(pane, &bytes) {
            Ok(true) => return Ran::ok(Vec::new()),
            Ok(false) => continue,
            Err(err) => return Ran::failed(format!("could not write to %{}: {err:#}", pane.get())),
        }
    }
    Ran::failed(format!("no pane %{}", pane.get()))
}

fn capture(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let Some(first) = args.first() else {
        return Ran::failed("usage: neotmux capture <pane> [--lines N]");
    };
    let Some(pane) = pane_id(first) else {
        return Ran::failed(format!("'{first}' is not a pane id"));
    };
    let back = match args.iter().position(|arg| arg == "--lines") {
        Some(at) => match args.get(at + 1).and_then(|n| n.parse::<usize>().ok()) {
            Some(n) => Some(n),
            None => return Ran::failed("--lines takes a number"),
        },
        None => None,
    };
    let want = Want::default();
    let Some(visible) = ctx.terminals.content(pane, None, want) else {
        return Ran::failed(format!("no pane %{}", pane.get()));
    };
    let Some(count) = back else {
        return Ran::ok(visible.lines);
    };
    // `--lines N` is the last N lines of everything the pane has: its history, then what is on
    // it now. A pane that has scrolled nothing answers with what it has.
    let extent = ctx.terminals.history_extent(pane).unwrap_or_default();
    let wanted_history = count.saturating_sub(visible.lines.len());
    let from = usize::try_from(extent.len)
        .unwrap_or(usize::MAX)
        .saturating_sub(wanted_history);
    let mut lines = match ctx.terminals.history(pane, from, wanted_history, want) {
        Some(history) => history.lines,
        None => Vec::new(),
    };
    lines.extend(visible.lines);
    let cut = lines.len().saturating_sub(count);
    Ran::ok(lines.split_off(cut))
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

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_trailing_enter_is_the_flag_and_is_not_in_the_text() {
        let (text, enter) = split_enter(&words(&["echo", "world", "--enter"]));
        assert_eq!(text, "echo world");
        assert!(enter);
    }

    #[test]
    fn an_enter_in_the_middle_is_payload_not_the_flag() {
        let (text, enter) = split_enter(&words(&["echo", "--enter", "world"]));
        assert_eq!(text, "echo --enter world");
        assert!(!enter);
    }

    #[test]
    fn a_bare_enter_sends_an_empty_line() {
        let (text, enter) = split_enter(&words(&["--enter"]));
        assert_eq!(text, "");
        assert!(enter);
    }

    #[test]
    fn no_enter_at_all_is_just_the_text() {
        let (text, enter) = split_enter(&words(&["echo", "world"]));
        assert_eq!(text, "echo world");
        assert!(!enter);
    }
}

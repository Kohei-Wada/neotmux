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
use neotmux_pty::PaneTerminals;
use neotmux_pty::child::Waker;

use crate::sessions::{rename_session, start_session, switch_clients};
use crate::{Attached, Plugins, RenameFailure, Session, Sessions, StartFailure};

/// What a command did.
pub(crate) struct Ran {
    pub(crate) ok: bool,
    /// stdout where `ok`, and the one sentence that says why not where it is not.
    pub(crate) lines: Vec<String>,
}

impl Ran {
    pub(crate) fn ok(lines: Vec<String>) -> Self {
        Self { ok: true, lines }
    }

    /// The one line a command answers with when it has nothing more to say than that it did.
    pub(crate) fn did(line: impl Into<String>) -> Self {
        Self {
            ok: true,
            lines: vec![line.into()],
        }
    }

    pub(crate) fn failed(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            lines: vec![why.into()],
        }
    }
}

/// Every name the host answers. The CLI reads it to tell a command from a session name.
pub const NAMES: &[&str] = &[
    "new", "rename", "windows", "panes", "send", "capture", "switch",
];

pub(crate) fn unknown(name: &str) -> Ran {
    Ran::failed(format!("unknown command '{name}'"))
}

/// What a command reaches into the server for. Built fresh at the door, out of whatever
/// `Serving` and the frame loop already hold.
pub(crate) struct Ctx<'a> {
    pub(crate) sessions: &'a mut Sessions,
    pub(crate) plugins: &'a mut Plugins,
    pub(crate) terminals: &'a Rc<PaneTerminals>,
    pub(crate) waker: &'a Waker,
    pub(crate) clients: &'a mut Vec<Attached>,
}

pub(crate) fn run(name: &str, args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    match name {
        "new" => new(args, session, ctx),
        "rename" => rename(args, ctx),
        "windows" => windows(args, session, ctx),
        "panes" => panes(args, session, ctx),
        "send" => send(args, session, ctx),
        "capture" => capture(args, session, ctx),
        "switch" => switch(args, ctx),
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
        .map_or_else(|| ctx.sessions.next_number(), str::to_string);
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

/// Moves whoever is attached to this server to the named session — `neotmux switch`'s whole
/// job, and what "bring me to that session" means when the thing asking is not the client.
fn switch(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let Some(name) = args.first() else {
        return Ran::failed("usage: neotmux switch <name>");
    };
    let Some(id) = ctx.sessions.id_of(name) else {
        return Ran::failed(format!("neotmux session '{name}' is not running"));
    };
    match switch_clients(id, None, ctx.clients, ctx.sessions, ctx.plugins) {
        Ok(0) => Ran::failed(format!("nothing is attached to move to '{name}'")),
        Ok(_) => Ran::did(name.clone()),
        Err(err) => Ran::failed(format!("could not switch to '{name}': {err:#}")),
    }
}

/// The session a command is addressed to, or the sentence that says there is none. Without
/// `-t` it is the only session there is: with more than one, which is meant has to be said.
fn addressed<'a>(session: Option<&str>, ctx: &'a mut Ctx<'_>) -> Result<&'a mut Session, Ran> {
    let only;
    let name = match session {
        Some(name) => name,
        None => {
            let names = ctx.sessions.names();
            match names.as_slice() {
                [one] => {
                    only = one.clone();
                    only.as_str()
                }
                [] => return Err(Ran::failed("no neotmux session is running")),
                _ => {
                    return Err(Ran::failed(format!(
                        "{} sessions are running; say which with -t <name>",
                        names.len()
                    )));
                }
            }
        }
    };
    match ctx.sessions.id_of(name) {
        Some(id) => Ok(ctx.sessions.get_mut(id).expect("a session it just named")),
        None => Err(Ran::failed(format!(
            "neotmux session '{name}' is not running"
        ))),
    }
}

/// The session a bare positional word names, where `-t` did not — the same fallback `new`
/// gives its name, because `windows` and `panes` take a session the same way `new`, `kill`
/// and `switch` do.
fn named(args: &[String], session: Option<&str>) -> Option<String> {
    args.first()
        .cloned()
        .or_else(|| session.map(str::to_string))
}

fn windows(args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let name = named(args, session);
    let held = match addressed(name.as_deref(), ctx) {
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

fn panes(args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let name = named(args, session);
    let terminals = ctx.terminals.clone();
    let held = match addressed(name.as_deref(), ctx) {
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

fn send(args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    if session.is_some() {
        return Ran::failed(
            "send takes a pane id and no -t: pane ids are the server's own and name a session already",
        );
    }
    let Some((first, rest)) = args.split_first() else {
        return Ran::failed("usage: neotmux send <pane> <text> [--enter]");
    };
    let Some(pane) = pane_id(first) else {
        return Ran::failed(format!("'{first}' is not a pane id"));
    };
    if rest.is_empty() {
        return Ran::failed("usage: neotmux send <pane> <text> [--enter]");
    }
    let (mut text, enter) = split_enter(rest);
    if enter {
        text.push('\r');
    }
    let bytes = text.clone().into_bytes();
    // Pane ids are the server's, so the pane is looked for in every session rather than in the
    // one `-t` named — which is why `send` takes no `-t` at all.
    for id in ctx.sessions.ids() {
        let holds = match ctx.sessions.get_mut(id) {
            Some(session) => session.holds_pane(pane),
            None => continue,
        };
        if !holds {
            continue;
        }
        // Asked once the owner is known, so a refusal names a pane that exists and a write
        // to a pane that does not is still the "no pane" answer it was.
        if let Err(why) = ctx.plugins.check_write(id, pane, &text, "tool") {
            return Ran::failed(why);
        }
        let Some(session) = ctx.sessions.get_mut(id) else {
            continue;
        };
        return match session.write_to_pane(pane, &bytes) {
            Ok(_) => Ran::ok(Vec::new()),
            Err(err) => Ran::failed(format!("could not write to %{}: {err:#}", pane.get())),
        };
    }
    Ran::failed(format!("no pane %{}", pane.get()))
}

/// The visible grid with its trailing blank rows dropped. A pane's blank bottom is not
/// content — a screen whose output sits at the top must not answer `--lines N` with N blank
/// rows, and a bare `capture` must not either. Interior blank lines stay; only the tail goes.
fn trim_trailing_blank(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().is_some_and(|line| line.trim_end().is_empty()) {
        lines.pop();
    }
    lines
}

fn capture(args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    if session.is_some() {
        return Ran::failed(
            "capture takes a pane id and no -t: pane ids are the server's own and name a session already",
        );
    }
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
    let visible = trim_trailing_blank(visible.lines);
    let Some(count) = back else {
        return Ran::ok(visible);
    };
    // `--lines N` is the last N lines of everything the pane has: its history, then what is on
    // it now. A pane that has scrolled nothing answers with what it has.
    let extent = ctx.terminals.history_extent(pane).unwrap_or_default();
    let wanted_history = count.saturating_sub(visible.len());
    let from = usize::try_from(extent.len)
        .unwrap_or(usize::MAX)
        .saturating_sub(wanted_history);
    let mut lines = match ctx.terminals.history(pane, from, wanted_history, want) {
        Some(history) => history.lines,
        None => Vec::new(),
    };
    lines.extend(visible);
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

    #[test]
    fn trailing_blank_rows_are_dropped_but_interior_ones_stay() {
        let lines = trim_trailing_blank(words(&["hello", "", "world", "", "", ""]));
        assert_eq!(
            lines,
            vec!["hello".to_string(), "".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn a_screen_with_nothing_on_it_trims_to_nothing() {
        assert!(trim_trailing_blank(words(&["", "  ", ""])).is_empty());
    }

    /// `NAMES` is what the CLI checks a word against before it is treated as a command;
    /// `run`'s match is what actually answers it. Nothing else checks the two agree, so a
    /// name added to one and not the other would silently fall through to `unknown` — this
    /// walks every name and asks that it does not.
    #[test]
    fn every_name_in_names_dispatches_to_something_other_than_unknown() {
        let layers = neotmux_lua::plugin::Layers::default();
        let terminals = Rc::new(PaneTerminals::new());
        let lua = neotmux_lua::LuaSubsystem::build(layers, terminals.clone()).expect("lua");
        let mut plugins = Plugins::new(lua);
        let mut sessions = Sessions::default();
        let waker = neotmux_pty::child::wakeup().expect("wakeup").1;
        let mut clients: Vec<Attached> = Vec::new();

        // `new` with no arguments spawns a real shell; a refused name reaches `run`'s match
        // the same as any other command without paying for one.
        let bad_name = vec!["-bad".to_string()];
        let empty: Vec<String> = Vec::new();

        for name in NAMES {
            let args = if *name == "new" { &bad_name } else { &empty };
            let mut ctx = Ctx {
                sessions: &mut sessions,
                plugins: &mut plugins,
                terminals: &terminals,
                waker: &waker,
                clients: &mut clients,
            };
            let ran = run(name, args, None, &mut ctx);
            assert_ne!(
                ran.lines.first().map(String::as_str),
                Some(format!("unknown command '{name}'")).as_deref(),
                "'{name}' fell through to unknown"
            );
        }
    }
}

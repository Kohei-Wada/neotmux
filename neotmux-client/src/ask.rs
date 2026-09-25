//! One question to the server, and what the answer prints.
//!
//! `ls`, `kill` and the host commands are the client too: they say a sentence over the
//! socket, read the one answer, and put it on stdout. There is no terminal in any of it —
//! that is [`crate::attach()`], the other thing a client can be asked to do.

use std::cell::RefCell;
use std::env;
use std::rc::Rc;

use anyhow::Context as _;

use crate::start;
use neotmux_sys::Link;
use neotmux_sys::sock;

/// Runs a host command and prints what it said. Lines on stdout where it worked, the one
/// sentence on stderr where it did not, and the exit status says which.
pub fn command(name: &str, args: &[String], session: Option<&str>) -> anyhow::Result<()> {
    let msg = neotmux_proto::wire::ToServer::Command {
        session: session.map(str::to_string),
        name: name.to_string(),
        args: args.to_vec(),
    };
    // `new` has nobody to ask yet where nothing is running — read as "nothing is running"
    // rather than as a question to start a server over, which is right for every other
    // command and wrong for this one, so it alone asks with a server started for it.
    let ran = match ask(&msg, name == "new")? {
        Some(answer) => answer,
        None => anyhow::bail!("no neotmux server is running"),
    };
    match ran {
        neotmux_proto::OwnedToClient::Ran { ok: true, lines } => {
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        neotmux_proto::OwnedToClient::Ran { ok: false, lines } => {
            anyhow::bail!("{}", lines.join("\n"))
        }
        other => anyhow::bail!("the neotmux server answered {other:?}"),
    }
}

/// Asks the server one question and reads its one answer. `None` is nothing listening, which
/// is not an error here: it is the answer to "what is running" — except when `start_if_needed`
/// says a server should be started rather than reported absent, which is right for `new` and
/// wrong for everything else that asks.
fn ask(
    msg: &neotmux_proto::wire::ToServer,
    start_if_needed: bool,
) -> anyhow::Result<Option<neotmux_proto::OwnedToClient>> {
    let link = if start_if_needed {
        Some(start::begin()?.finish()?)
    } else {
        let paths = sock::Paths::resolve(&|key| env::var_os(key))?;
        match sock::connect(&paths.socket)? {
            Some(stream) => Some(Rc::new(RefCell::new(
                Link::open(stream).context("greet the neotmux server")?,
            ))),
            None => None,
        }
    };
    let Some(link) = link else {
        return Ok(None);
    };
    let mut peer = link.borrow_mut();
    peer.send(&neotmux_proto::wire::encode_to_server(msg))
        .context("ask the neotmux server")?;
    let Some(message) = peer.recv().context("read the server's answer")? else {
        anyhow::bail!("the neotmux server went away without answering");
    };
    Ok(Some(
        neotmux_proto::wire::decode_to_client(&message).context("decode the server's answer")?,
    ))
}

/// The pid of the server, out of its lock file: the process a person would signal, and the
/// one `ls` has always put on the line.
fn server_pid() -> Option<i32> {
    let paths = sock::Paths::resolve(&|key| env::var_os(key)).ok()?;
    std::fs::read_to_string(paths.lock)
        .ok()?
        .trim()
        .parse::<i32>()
        .ok()
}

/// Ends one session, by asking the server holding it. It used to be a signal to a process,
/// because a session was one and there was no protocol to ask through. There is now.
pub fn kill(name: &str) -> anyhow::Result<()> {
    let asked = ask(
        &neotmux_proto::wire::ToServer::Kill {
            session: name.to_string(),
        },
        false,
    )?;
    match asked {
        Some(neotmux_proto::OwnedToClient::Killed(true)) => {}
        Some(neotmux_proto::OwnedToClient::Killed(false)) | None => {
            anyhow::bail!("neotmux session '{name}' is not running")
        }
        Some(other) => anyhow::bail!("the neotmux server answered {other:?}"),
    }
    match server_pid() {
        Some(pid) => println!("neotmux: asked the session '{name}' (pid {pid}) to end"),
        None => println!("neotmux: asked the session '{name}' to end"),
    }
    Ok(())
}

/// The running sessions, the way the server says they are. It used to be a directory walk and
/// a connect probe per file, because the sessions were processes; they are one server's now,
/// and it is asked.
pub fn ls() -> anyhow::Result<()> {
    let names = match ask(&neotmux_proto::wire::ToServer::List, false)? {
        Some(neotmux_proto::OwnedToClient::Sessions(names)) => names,
        Some(other) => anyhow::bail!("the neotmux server answered {other:?}"),
        None => Vec::new(),
    };
    if names.is_empty() {
        anyhow::bail!("no neotmux sessions are running");
    }
    let pid = server_pid();
    for name in names {
        match pid {
            Some(pid) => println!("{name}  pid {pid}"),
            None => println!("{name}"),
        }
    }
    Ok(())
}

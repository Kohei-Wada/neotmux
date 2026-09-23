//! Getting the two processes into the same conversation.
//!
//! `neotmux` with no arguments is a client. If something is listening on the session's
//! socket it talks to that; if nothing is, it starts a copy of itself with `--server` and
//! waits for it to come up. Which of those it is doing is decided by connecting, because
//! that is the only question about a unix socket with no race in it — see [`neotmux::sock`].

use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Context as _;

use neotmux::link::Link;
use neotmux::sock::{self, DEFAULT_SESSION, Lock, Paths};

use crate::server;
use crate::signals::ShutdownSignal;

/// How long a client waits for a server it started to come up. Generous, because it is a
/// fork and an exec on a machine that may be busy, and it only ever runs out when something
/// is actually wrong.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// The server, as `--server` runs it.
///
/// It stands down quietly rather than complaining when somebody else got there first: two
/// clients starting at the same instant will both have tried, and the loser has nothing to
/// report — its client is about to connect to the winner.
pub fn serve(session: Option<String>) -> anyhow::Result<()> {
    let name = session.as_deref().unwrap_or(DEFAULT_SESSION);
    let paths = Paths::resolve(&|key| std::env::var_os(key), name)?;
    let Some(_lock) = Lock::take(&paths)? else {
        return Ok(());
    };
    let Some(listener) = sock::bind(&paths)? else {
        return Ok(());
    };
    let shutdown = ShutdownSignal::install()?;
    let served = server::serve(&listener, &shutdown);
    // Taken away on the way out, so the next client does not connect to a server that is
    // gone — and only if it is still this server's, so one that outlived a replacement does
    // not take the replacement's name with it.
    sock::unbind(&paths, &listener);
    // The server's standard error goes to `/dev/null` — it has no screen to complain to —
    // so the log is the only place a failure can be read afterwards.
    if let Err(err) = &served {
        neotmux::logging::log(
            neotmux_proto::Level::Error,
            "neotmux-server",
            &format!("the session ended in an error: {err:#}"),
        );
    }
    served
}

/// A server being started, or one that was already there.
///
/// Split in two so that the client can get on with whatever else it has to do while the
/// server forks, execs and binds. That used to be loading the plugins; the plugins are the
/// server's now, and what is left is the terminal.
pub struct Starting {
    paths: Paths,
    connected: Option<UnixStream>,
}

/// Finds this session's server, or starts one, and returns without waiting for it.
pub fn begin(session: Option<String>) -> anyhow::Result<Starting> {
    let name = session.as_deref().unwrap_or(DEFAULT_SESSION);
    let paths = Paths::resolve(&|key| std::env::var_os(key), name)?;
    let connected = sock::connect(&paths.socket)?;
    if connected.is_none() {
        spawn(session)?;
    }
    Ok(Starting { paths, connected })
}

impl Starting {
    /// The connection, once there is one to have.
    pub fn finish(self) -> anyhow::Result<Rc<RefCell<Link>>> {
        if let Some(stream) = self.connected {
            return open(stream);
        }
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if let Some(stream) = sock::connect(&self.paths.socket)? {
                return open(stream);
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "the neotmux server did not come up on {}",
                self.paths.socket.display()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn open(stream: UnixStream) -> anyhow::Result<Rc<RefCell<Link>>> {
    // The version check is `decode_hello`'s, inside `Link::open`, and its message names both
    // numbers. Everything else about a failed greeting is an ordinary I/O error.
    let link = Link::open(stream).context("greet the neotmux server")?;
    Ok(Rc::new(RefCell::new(link)))
}

/// Starts the other half of this binary.
///
/// `setsid` rather than a plain fork: the server must not share the client's controlling
/// terminal, or the `C-c` meant for a shell in a pane would reach the process holding every
/// pane. Its standard descriptors go to `/dev/null` for the same reason — it has nothing to
/// say to a screen it does not own, and the log file is where it says it.
fn spawn(session: Option<String>) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("find the neotmux binary")?;
    // No `current_dir` on the server: its own directory is the startup directory, and
    // every pane that is not told where to go from asks one of its children.
    let mut command = Command::new(exe);
    command.arg("--server");
    if let Some(name) = session {
        command.arg(name);
    }
    if let Some(log) = logged() {
        command.arg("--log").arg(log);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `fork`, `_exit` and `setsid` are all async-signal-safe and touch nothing this
    // process allocated.
    unsafe {
        command.pre_exec(|| {
            // Forked twice, so the process that execs the server is not this client's child:
            // a client that outlives its server would otherwise have to reap it, and one
            // that does not would leave a zombie behind.
            if let nix::unistd::ForkResult::Parent { .. } = nix::unistd::fork()? {
                nix::libc::_exit(0);
            }
            nix::unistd::setsid()?;
            Ok(())
        });
    }
    let mut child = command.spawn().context("start the neotmux server")?;
    // The middle process, which is already on its way out.
    let _ = child.wait();
    Ok(())
}

/// The `--log` this client was given, so the server writes to the same file.
fn logged() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--log" {
            return args.next().filter(|value| !value.starts_with('-'));
        }
    }
    None
}

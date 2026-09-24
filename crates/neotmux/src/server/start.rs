//! The `--server` half's own beginning.
//!
//! Everything a session needs before there is one: the lock that says this process is the
//! server, the socket it binds, and the taking of both away on the way out. The loop itself
//! is [`super::serve`]; this is only how it is entered and left.

use crate::signals::ShutdownSignal;
use crate::sock::{self, Lock, Paths};

/// The server, as `--server` runs it.
///
/// It stands down quietly rather than complaining when somebody else got there first: two
/// clients starting at the same instant will both have tried, and the loser has nothing to
/// report — its client is about to connect to the winner.
pub fn serve() -> anyhow::Result<()> {
    let paths = Paths::resolve(&|key| std::env::var_os(key))?;
    let Some(_lock) = Lock::take(&paths)? else {
        return Ok(());
    };
    let Some(listener) = sock::bind(&paths)? else {
        return Ok(());
    };
    let shutdown = ShutdownSignal::install()?;
    let served = super::serve(&listener, &shutdown);
    // Taken away on the way out, so the next client does not connect to a server that is
    // gone — and only if it is still this server's, so one that outlived a replacement does
    // not take the replacement's name with it.
    sock::unbind(&paths, &listener);
    // The server's standard error goes to `/dev/null` — it has no screen to complain to —
    // so the log is the only place a failure can be read afterwards.
    if let Err(err) = &served {
        crate::logging::log(
            neotmux_proto::Level::Error,
            "neotmux-server",
            &format!("the session ended in an error: {err:#}"),
        );
    }
    served
}

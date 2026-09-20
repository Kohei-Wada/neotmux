//! Shared by every PTY-driven test harness that starts a real neotmux session: without this,
//! the session Task 1 made immortal outlives the test that started it, and a full suite run
//! leaves a server and a live shell behind for every such test.

use std::path::Path;

/// Ends the session behind `runtime_dir` — a `SIGTERM` to the pid its lock file names, taken
/// only when the lock is actually held, the same rule `neotmux --kill` follows. `runtime_dir`
/// must be the test's own isolated `$XDG_RUNTIME_DIR`; nothing else is ever safe to signal
/// this way.
pub(crate) fn end_session(runtime_dir: &Path) {
    let runtime_dir = runtime_dir.to_owned();
    let Ok(paths) = neotmux::sock::Paths::resolve(&move |key| {
        (key == "XDG_RUNTIME_DIR").then(|| runtime_dir.clone().into_os_string())
    }) else {
        return;
    };
    if let Ok(Some(pid)) = neotmux::sock::holder(&paths) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
}

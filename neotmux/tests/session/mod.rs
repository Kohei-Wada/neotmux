//! Shared by every PTY-driven test harness that starts a real neotmux session: without this,
//! the sessions a test started outlive it — they are immortal now — and a full suite run
//! leaves a server and a live shell behind for every such test.

use std::path::Path;

/// Ends the server behind `runtime_dir`, and with it every session it holds — a `SIGTERM` to
/// the pid its lock file names, taken only when the lock is actually held. `runtime_dir` must
/// be the test's own isolated `$XDG_RUNTIME_DIR`; nothing else is ever safe to signal this
/// way.
pub(crate) fn end_session(runtime_dir: &Path) {
    let runtime_dir = runtime_dir.to_owned();
    let Ok(paths) = neotmux_sys::sock::Paths::resolve(&move |key| {
        (key == "XDG_RUNTIME_DIR").then(|| runtime_dir.clone().into_os_string())
    }) else {
        return;
    };
    let pid = match neotmux_sys::sock::holder(&paths) {
        Ok(neotmux_sys::sock::Holder::Pid(pid)) => pid,
        // `holder` confirms through `/proc`, which only Linux has; elsewhere `ps` confirms.
        Ok(neotmux_sys::sock::Holder::Unconfirmed) => match confirmed_by_ps(&paths.lock) {
            Some(pid) => pid,
            None => return,
        },
        _ => return,
    };
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    );
}

/// The pid in a held lock, if `ps` names it neotmux.
fn confirmed_by_ps(lock: &Path) -> Option<i32> {
    let pid = std::fs::read_to_string(lock)
        .ok()?
        .trim()
        .parse::<i32>()
        .ok()?;
    let out = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let comm = String::from_utf8_lossy(&out.stdout);
    (Path::new(comm.trim()).file_name()? == "neotmux").then_some(pid)
}

/// Ends the session behind a test's own `$XDG_RUNTIME_DIR` when dropped — on a panic as much
/// as on a normal return. A bare `end_session(...)` at the end of a function is skipped by an
/// assertion that fails above it, and the session it would have ended used to die in three
/// seconds; now it lives until the machine reboots.
pub(crate) struct EndsSession(std::path::PathBuf);

impl EndsSession {
    pub(crate) fn new(runtime_dir: &Path) -> Self {
        Self(runtime_dir.to_owned())
    }
}

impl Drop for EndsSession {
    fn drop(&mut self) {
        end_session(&self.0);
    }
}

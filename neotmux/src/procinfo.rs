//! Linux only: everything here is `/proc`, and on any other platform a pane's facts are
//! simply unknown. There is no fallback, and pretending otherwise would be a lie in the one
//! place a user would believe it.

use std::fs;

use neotmux_proto::PaneInfo;

/// What `/proc` says about one process. Never fails: the process can exit between the caller
/// deciding to ask and this reading the file, and a status line must not start raising
/// because a shell went away.
pub fn read(pid: i32) -> PaneInfo {
    if pid <= 0 {
        return PaneInfo::default();
    }
    PaneInfo {
        command: command(pid),
        cwd: cwd(pid),
    }
}

/// `comm` rather than `cmdline`: the status line wants the name of the thing running, and
/// `comm` is already that — no arguments, no NUL splitting, no quoting to undo.
pub(crate) fn command(pid: i32) -> Option<String> {
    let text = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = text.trim_end_matches('\n');
    (!name.is_empty()).then(|| name.to_string())
}

/// The process's working directory: `/proc/<pid>/cwd`, `None` when the process is gone.
/// A deleted directory is a different answer: the link still reads, but as `"<path> (deleted)"` —
/// a string no one can open. It is the caller's job to reject that (`Session::start_dir`'s
/// `is_dir` check does exactly that), and it is that check, not a `None` here, that keeps a
/// deleted directory from becoming a pane's starting point.
pub(crate) fn cwd(pid: i32) -> Option<String> {
    let path = fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    Some(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_reports_its_own_name_and_directory() {
        let me = std::process::id() as i32;
        let info = read(me);
        let command = info.command.expect("this process has a comm");
        assert!(
            command.starts_with("procinfo") || command.starts_with("neotmux"),
            "unexpected comm for the test binary: {command}"
        );
        assert_eq!(
            info.cwd.map(std::path::PathBuf::from),
            std::env::current_dir().ok()
        );
    }

    /// A process that has exited is the ordinary case, not an error: nothing is known about
    /// it and nothing is raised.
    #[test]
    fn a_pid_that_does_not_exist_is_unknown_rather_than_an_error() {
        // The highest pid Linux will hand out is bounded by pid_max, which cannot exceed
        // 2^22; one past that can never name a live process.
        assert!(read(1 << 23).is_unknown());
    }

    #[test]
    fn a_nonsense_pid_is_unknown() {
        assert!(read(0).is_unknown());
        assert!(read(-1).is_unknown());
    }
}

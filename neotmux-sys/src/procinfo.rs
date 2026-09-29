//! Linux only: everything here is `/proc`, and on any other platform a pane's facts are
//! simply unknown. There is no fallback, and pretending otherwise would be a lie in the one
//! place a user would believe it.

use std::collections::{HashSet, VecDeque};
use std::fs;

use neotmux_proto::{Job, JobProcess, PaneInfo};

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
        job: job(pid),
    }
}

/// `comm` rather than `cmdline`: the status line wants the name of the thing running, and
/// `comm` is already that — no arguments, no NUL splitting, no quoting to undo.
pub(crate) fn command(pid: i32) -> Option<String> {
    let text = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = text.trim_end_matches('\n');
    (!name.is_empty()).then(|| name.to_string())
}

/// How many processes one walk of a job visits: a shell that has piled up descendants must not
/// make every refresh read without limit.
const JOB_WALK_LIMIT: usize = 512;

/// The foreground job `group` leads: every process reachable from the leader through
/// `/proc/<pid>/task/<tid>/children` that is still in the group. A wrapper's argv is where the
/// program it is carrying is named, so each member's is read — except where the process is
/// exiting or in uninterruptible sleep, when reading its memory can block.
pub fn job(group: i32) -> Option<Job> {
    if group <= 0 {
        return None;
    }
    let mut seen = HashSet::from([group]);
    let mut queue = VecDeque::from([group]);
    let mut processes = Vec::new();
    let mut walked = 0;
    while let Some(pid) = queue.pop_front() {
        if walked == JOB_WALK_LIMIT {
            break;
        }
        walked += 1;
        if let Some((name, state, pgrp)) = stat(pid)
            && pgrp == group
        {
            let argv = (!matches!(state, 'D' | 'Z' | 'X' | 'x'))
                .then(|| argv(pid))
                .flatten();
            processes.push(JobProcess { pid, name, argv });
        }
        for child in children(pid) {
            if seen.insert(child) {
                queue.push_back(child);
            }
        }
    }
    processes.sort_unstable_by_key(|process| process.pid);
    (!processes.is_empty()).then_some(Job { group, processes })
}

/// `comm`, state and process group, out of `/proc/<pid>/stat`. The `comm` is everything
/// between the first `(` and the last `)`: it can hold either.
fn stat(pid: i32) -> Option<(String, char, i32)> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let name = text.get(open + 1..close)?.to_string();
    let mut fields = text.get(close + 1..)?.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _ppid = fields.next()?;
    let pgrp = fields.next()?.parse().ok()?;
    Some((name, state, pgrp))
}

fn children(pid: i32) -> Vec<i32> {
    let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for task in tasks.flatten() {
        let Ok(text) = fs::read_to_string(task.path().join("children")) else {
            continue;
        };
        out.extend(
            text.split_whitespace()
                .filter_map(|c| c.parse::<i32>().ok()),
        );
    }
    out
}

fn argv(pid: i32) -> Option<Vec<String>> {
    let bytes = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let parts: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    (!parts.is_empty()).then_some(parts)
}

/// The process's working directory: `/proc/<pid>/cwd`, `None` when the process is gone.
/// A deleted directory is a different answer: the link still reads, but as `"<path> (deleted)"` —
/// a string no one can open. It is the caller's job to reject that (`Session::start_dir`'s
/// `is_dir` check does exactly that), and it is that check, not a `None` here, that keeps a
/// deleted directory from becoming a pane's starting point.
pub fn cwd(pid: i32) -> Option<String> {
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

    /// A real job: `sh` running a script in a process group of its own, with a `sleep` under
    /// it. Both are in the job, and the leader's argv names the script it was handed.
    #[test]
    fn a_job_is_its_leader_and_what_it_started() {
        use std::os::unix::process::CommandExt;

        let dir = tempfile::tempdir().expect("scratch dir");
        let script = dir.path().join("codex");
        std::fs::write(&script, "sleep 30; true\n").expect("script");
        let mut child = std::process::Command::new("/bin/sh")
            .arg(&script)
            .process_group(0)
            .spawn()
            .expect("sh");
        let group = child.id() as i32;

        // Until one read has both: the `sleep` in the job, and the leader's argv, which is not
        // read while the leader is in a state where reading it could block — as it can be for
        // a moment while it forks the `sleep`.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut found = None;
        while std::time::Instant::now() < deadline {
            found = job(group).filter(|job| {
                job.processes.iter().any(|p| p.name == "sleep")
                    && job
                        .processes
                        .iter()
                        .any(|p| p.pid == group && p.argv.is_some())
            });
            if found.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(group),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = child.wait();

        let job = found.expect("no read of the job had its sleep and its leader's argv");
        assert_eq!(job.group, group);
        let leader = job
            .processes
            .iter()
            .find(|p| p.pid == group)
            .expect("the leader");
        assert_eq!(
            leader.argv.as_deref(),
            Some(&["/bin/sh".to_string(), script.to_string_lossy().into_owned()][..])
        );
        assert!(job.processes.iter().any(|p| p.name == "sleep"), "{job:?}");
    }

    #[test]
    fn a_nonsense_pid_is_unknown() {
        assert!(read(0).is_unknown());
        assert!(read(-1).is_unknown());
    }
}

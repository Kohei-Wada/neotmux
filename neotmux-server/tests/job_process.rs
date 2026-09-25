//! One real process, to prove the seam is wired to something that exists. Everything about
//! what a plugin sees is in `neotmux-lua`'s own tests, against a fake.

use std::time::{Duration, Instant};

use neotmux_lua::host::{JobChunk, JobSpec, Runner};

#[test]
fn a_real_process_runs_and_its_output_comes_back() {
    let (wakeup, waker) = neotmux_pty::child::wakeup().unwrap();
    let runner = neotmux_server::Processes::new(waker);
    let mut running = runner
        .start(&JobSpec {
            // `sh -c`, not `/bin/echo`: not every system has one — NixOS does not — and
            // `/bin/sh` is the one path POSIX actually promises.
            cmd: vec!["/bin/sh".into(), "-c".into(), "echo hello".into()],
            cwd: None,
            env: Vec::new(),
        })
        .expect("/bin/sh did not start");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    let mut code = None;
    while Instant::now() < deadline && code.is_none() {
        for chunk in running.poll() {
            match chunk {
                JobChunk::Stdout(bytes) => out.extend_from_slice(&bytes),
                JobChunk::Exited(status) => code = Some(status),
                JobChunk::Failed(why) => panic!("{why}"),
                JobChunk::Stderr(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(code, Some(0), "the job did not exit");
    assert_eq!(String::from_utf8_lossy(&out).trim(), "hello");
    drop(wakeup);
}

/// A kill reaches what the program started, not only the program. The `sleep` is a grandchild
/// holding the write end of the pipe this job is read through: signalling only the `sh` would
/// leave both reader threads blocked in `read` and the exit never reported at all.
#[test]
fn killing_a_job_ends_the_children_it_started() {
    let (wakeup, waker) = neotmux_pty::child::wakeup().unwrap();
    let runner = neotmux_server::Processes::new(waker);
    let mut running = runner
        .start(&JobSpec {
            cmd: vec!["/bin/sh".into(), "-c".into(), "sleep 300 | cat".into()],
            cwd: None,
            env: Vec::new(),
        })
        .expect("/bin/sh did not start");

    // Long enough for the shell to have started the pipeline, so the kill has grandchildren
    // to reach rather than arriving before there were any.
    std::thread::sleep(Duration::from_millis(200));
    running.kill();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ended = false;
    while Instant::now() < deadline && !ended {
        for chunk in running.poll() {
            match chunk {
                JobChunk::Exited(_) | JobChunk::Failed(_) => ended = true,
                JobChunk::Stdout(_) | JobChunk::Stderr(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        ended,
        "the exit never arrived: something the job started is still holding a pipe open"
    );
    drop(wakeup);
}

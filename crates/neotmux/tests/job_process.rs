//! One real process, to prove the seam is wired to something that exists. Everything about
//! what a plugin sees is in `neotmux-lua`'s own tests, against a fake.

use std::time::{Duration, Instant};

use neotmux_lua::runner::{JobChunk, JobSpec, Runner};

#[test]
fn a_real_process_runs_and_its_output_comes_back() {
    let (wakeup, waker) = neotmux_term::wakeup::wakeup().unwrap();
    let runner = neotmux::runner::Processes::new(waker);
    let mut running = runner
        .start(&JobSpec {
            cmd: vec!["/bin/echo".into(), "hello".into()],
            cwd: None,
            env: Vec::new(),
        })
        .expect("/bin/echo did not start");

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

    assert_eq!(code, Some(0), "/bin/echo did not exit");
    assert_eq!(String::from_utf8_lossy(&out).trim(), "hello");
    drop(wakeup);
}

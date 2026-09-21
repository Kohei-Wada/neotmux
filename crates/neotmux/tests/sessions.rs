//! Named sessions over the binary: `work` starts beside `default`, `ls` says which of
//! them is running, `kill work` ends only `work`, and a name that would not be a file in
//! the directory is refused before it is a file.

use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

mod harness;
mod session;

use harness::Neotmux;

/// The test's own `$XDG_RUNTIME_DIR`. `_end` is declared first so it drops first — ended
/// whatever session a test left behind while the directory its lock is in still exists,
/// the order `socket.rs`'s TestRuntime already keeps.
struct Runtime {
    _end: session::EndsSession,
    dir: tempfile::TempDir,
}

impl Runtime {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("runtime dir");
        let _end = session::EndsSession::new(dir.path());
        Self { _end, dir }
    }
}

fn bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_neotmux"))
}

/// `kill` signals the server and the server notices on its own loop; the socket stops
/// answering the moment that effect lands. Wait for it rather than asserting on the race
/// between the signal and the next line — the same wait `socket.rs` makes after its own
/// `kill` (`kill_ends_a_session_nobody_is_attached_to`).
fn wait_for_gone(rt: &Runtime, name: &str) {
    let socket = rt.dir.path().join("neotmux").join(format!("{name}.sock"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while UnixStream::connect(&socket).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the session '{name}' outlived kill"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The client takes the terminal before its server has bound — the two run at the same time — so a
/// test that uses the session waits for the binding first: the socket to be answerable.
/// The same wait `socket.rs` makes in its `started()`, in reverse.
fn up(rt: &Runtime, name: &str) {
    let socket = rt.dir.path().join("neotmux").join(format!("{name}.sock"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while UnixStream::connect(&socket).is_err() {
        assert!(
            Instant::now() < deadline,
            "the session '{name}' never came up"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `ls` is a connect-probe of whichever directory it is told about, so it runs under the
/// test's, and it is read as words, one session per line, name first.
fn ls(rt: &Runtime) -> (i32, String, String) {
    let out = Command::new(bin())
        .env("XDG_RUNTIME_DIR", rt.dir.path())
        .env("HOME", rt.dir.path())
        .arg("ls")
        .output()
        .expect("run neotmux ls");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn kill(rt: &Runtime, name: Option<&str>) -> (i32, String) {
    let mut cmd = Command::new(bin());
    cmd.env("XDG_RUNTIME_DIR", rt.dir.path())
        .env("HOME", rt.dir.path());
    cmd.arg("kill");
    if let Some(name) = name {
        cmd.arg(name);
    }
    let out = cmd.output().expect("run neotmux kill");
    // Both streams: a refusal is said on stderr (the way the sibling `socket.rs` test
    // reads it), a success on stdout, and we want whichever the case puts where.
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn a_named_session_starts_and_ls_shows_only_what_is_running() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);
    let _nt = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    up(&rt, "work");

    let (code, out, err) = ls(&rt);
    assert_eq!(
        code, 0,
        "one session running, and ls says so: {out:?} {err:?}"
    );
    assert!(
        out.lines()
            .any(|l| l.starts_with("work") && l.contains("pid ")),
        "work is running, with its server's pid on the line the lock file gave it: {out:?}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("default")),
        "default was never started in this test's dir: {out:?}"
    );
}

#[test]
fn a_second_client_of_the_same_name_gets_the_first_client_s_pane() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);

    let mut first = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    first.settle(Duration::from_millis(300));
    first.send("hello-two\r");
    first.wait_for_screen("hello-two", Duration::from_secs(10));
    first.kill(); // the client goes; the pane stays, the way a detach already proves, with a name on it

    let mut second = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    second.wait_for_screen("hello-two", Duration::from_secs(10));
    // And the second one is not handed a fresh session: a fresh one would be an empty
    // shell that has never seen `hello-two`, which is to say this wait would never end.
}

#[test]
fn kill_work_leaves_default_running_and_vica_versa() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);
    let a = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    let b = Neotmux::start_named_on(rt.dir.path(), "default", &layer, "/bin/cat");
    let (_a, _b) = (a, b); // both held until the end, for the drop order at the bottom
    up(&rt, "work");
    up(&rt, "default");

    let (code, out) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "kill work: {out:?}");
    wait_for_gone(&rt, "work");

    let (code, out, err) = ls(&rt);
    assert_eq!(code, 0, "default is still running: {out:?} {err:?}");
    assert!(
        out.lines().any(|l| l.starts_with("default")),
        "default survived kill work: {out:?}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("work")),
        "work is gone: {out:?}"
    );
}

#[test]
fn a_name_that_would_not_be_a_file_is_refused_and_leaves_nothing() {
    let rt = Runtime::new();
    // `a:b` is the spec's; `ls work` is the one the Review Focus names. All four are
    // refusals, and they are said, not warned.
    for args in [
        vec!["kill", "a:b"],
        vec!["kill", "-lead"],
        vec!["a:b"],
        vec!["ls", "work"],
    ] {
        let out = Command::new(bin())
            .env("XDG_RUNTIME_DIR", rt.dir.path())
            .env("HOME", rt.dir.path())
            .args(args.iter().copied())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("run");
        let code = out.status.code().unwrap_or(-1);
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_ne!(
            code, 0,
            "a refused invocation was accepted: {args:?} — {said:?}"
        );
    }
    // Neither one is a file in the directory: the refusal was at the argument, which is
    // what keeps this from being a leak.
    let leaked: Vec<String> = std::fs::read_dir(rt.dir.path().join("neotmux"))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains(':') || n.starts_with('-'))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leaked.is_empty(),
        "a refused name left a file in {}'s dir: {leaked:?}",
        rt.dir.path().display()
    );
}

#[test]
fn kill_says_which_name_it_is_refusing_to_guess_at() {
    let rt = Runtime::new();

    let (no_code, said) = kill(&rt, Some("nowhere"));
    assert_ne!(
        no_code, 0,
        "nothing running, and kill says it is not running: {said:?}"
    );
    assert!(
        said.contains("not running"),
        "the refusal names the case: {said:?}"
    );

    let layer = harness::runtime(&[("init.lua", "")]);
    let _nt = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    up(&rt, "work");
    let (code, said) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "kill work: {said:?}");
    wait_for_gone(&rt, "work");
    let (code, out, err) = ls(&rt);
    assert_eq!(
        code, 1,
        "work is gone, and ls with nothing running exits 1: {out:?} {err:?}"
    );
}

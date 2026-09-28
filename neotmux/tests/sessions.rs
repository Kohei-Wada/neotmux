//! Named sessions over the binary: `work` starts beside `default`, `ls` says which of
//! them is running, `kill work` ends only `work`, and a name that would not be a file in
//! the directory is refused before it is a file.

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

/// Whether the server says this session is running. A session is not a file any more — one
/// socket holds all of them — so its existence is a question rather than a path to stat.
fn running(rt: &Runtime, name: &str) -> bool {
    let (code, out, _) = ls(rt);
    code == 0 && out.lines().any(|line| line.starts_with(name))
}

/// `kill` reaches the server and the server ends the session on its own loop; `ls` stops
/// naming it the moment that effect lands. Waited for rather than asserted on the race
/// between the ask and the next line.
fn wait_for_gone(rt: &Runtime, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while running(rt, name) {
        assert!(
            Instant::now() < deadline,
            "the session '{name}' outlived kill"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The client takes the terminal before its server has bound — the two run at the same time —
/// so a test that uses the session waits for it to exist first.
fn up(rt: &Runtime, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !running(rt, name) {
        assert!(
            Instant::now() < deadline,
            "the session '{name}' never came up"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `ls` asks the server under whichever runtime directory it is told about, and is read as
/// words: one session per line, name first.
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

    // The test ends the session it started: `Runtime`'s drop ends only the default one, and
    // a live server is what this test is about, not what it may leave behind.
    let (code, said) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "kill work: {said:?}");
    wait_for_gone(&rt, "work");
}

#[test]
fn a_second_client_of_the_same_name_gets_the_first_client_s_pane() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);

    let mut first = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    first.send("hello-two\r");
    first.wait_for_screen("hello-two", Duration::from_secs(10));
    first.kill(); // the client goes; the pane stays, the way a detach already proves, with a name on it

    let mut second = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    second.wait_for_screen("hello-two", Duration::from_secs(10));
    // And the second one is not handed a fresh session: a fresh one would be an empty
    // shell that has never seen `hello-two`, which is to say this wait would never end.

    // And the test takes the session home with it, the way `a_named_session_starts_...`
    // does: both clients are gone here, and `Runtime`'s drop ends only the default one.
    let (code, said) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "kill work: {said:?}");
    wait_for_gone(&rt, "work");
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
    let (code, said) = kill(&rt, Some("default"));
    assert_eq!(
        code, 0,
        "default goes too, and the test is done with it: {said:?}"
    );
    wait_for_gone(&rt, "default");

    // The other way, on a fresh pair, because the state it is caught in is the one where
    // both are alive: a `kill default` that signalled `work`'s pid dies on that
    // assertion, and not quietly after `work` had already been the one that died.
    let a = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    let b = Neotmux::start_named_on(rt.dir.path(), "default", &layer, "/bin/cat");
    let (_a, _b) = (a, b); // both held until the end, the way the first pair was
    up(&rt, "work");
    up(&rt, "default");

    let (code, said) = kill(&rt, Some("default"));
    assert_eq!(code, 0, "kill default: {said:?}");
    wait_for_gone(&rt, "default");

    let (code, out, err) = ls(&rt);
    assert_eq!(code, 0, "work is still running: {out:?} {err:?}");
    assert!(
        out.lines().any(|l| l.starts_with("work")),
        "work survived kill default: {out:?}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("default")),
        "default is gone: {out:?}"
    );
    let (code, said) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "and the pair's work goes with the pair: {said:?}");
    wait_for_gone(&rt, "work");
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

/// `kill work` with two terminals in the session ends it for both of them, exactly as it
/// ends it for the one. The regression it catches runs in either direction: a kill that
/// reached only the client it found first, and a kill that stopped reaching any of them
/// once a session could have more than one.
#[test]
fn kill_work_with_two_clients_attached_ends_both() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);

    let mut first = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    first.send("two-are-looking\r");
    first.wait_for_screen("two-are-looking", Duration::from_secs(10));

    // The second terminal is in the same session, not a fresh one: what it is shown is what
    // the first one typed, which an empty shell of its own would never say.
    let mut second = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    second.wait_for_screen("two-are-looking", Duration::from_secs(10));
    up(&rt, "work");

    let (code, said) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "kill work: {said:?}");
    wait_for_gone(&rt, "work");

    assert!(
        first.wait_for_exit(Duration::from_secs(10)).is_some(),
        "the first terminal is out: a killed session is one no client is left in"
    );
    assert!(
        second.wait_for_exit(Duration::from_secs(10)).is_some(),
        "and so is the second, which is the half a one-client kill could not have said"
    );
}

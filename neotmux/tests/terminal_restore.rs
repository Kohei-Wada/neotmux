use std::io::Read;
use std::time::{Duration, Instant};

use nix::sys::signal::{self, Signal};
use nix::sys::termios::{LocalFlags, Termios};
use nix::unistd::Pid;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

mod session;

fn run_in_pty(env: &[(&str, &str)], input: &[u8]) -> (String, bool, Termios) {
    run_in_pty_sized(24, 80, env, input)
}

fn run_in_pty_sized(
    rows: u16,
    cols: u16,
    env: &[(&str, &str)],
    input: &[u8],
) -> (String, bool, Termios) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let isolation = tempfile::tempdir().expect("isolation dir");
    let _end_session = session::EndsSession::new(isolation.path());
    let log = isolation.path().join("neotmux.log");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
    cmd.env("XDG_RUNTIME_DIR", isolation.path());
    cmd.arg("--log");
    cmd.arg(&log);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    if !input.is_empty() {
        std::thread::sleep(Duration::from_millis(500));
        use std::io::Write as _;
        writer.write_all(input).expect("write");
        writer.flush().expect("flush");
    }

    let collector = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        out
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            break s;
        }
        assert!(Instant::now() < deadline, "neotmux did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    let termios = pair.master.get_termios().expect("termios after exit");
    drop(pair.master);
    let out = collector.join().expect("collector");
    (
        String::from_utf8_lossy(&out).into_owned(),
        status.success(),
        termios,
    )
}

fn assert_termios_restored(termios: &Termios) {
    assert!(
        termios.local_flags.contains(LocalFlags::ECHO),
        "echo was not restored"
    );
    assert!(
        termios.local_flags.contains(LocalFlags::ICANON),
        "icanon was not restored"
    );
}

#[test]
fn leaves_the_alternate_screen_on_clean_exit() {
    let (out, ok, termios) = run_in_pty(&[("SHELL", "/bin/sh")], b"exit\r");
    assert!(ok, "expected a clean exit");
    assert!(
        out.contains("\x1b[?1049l"),
        "alternate screen was never left"
    );
    assert_termios_restored(&termios);
}

#[test]
#[cfg_attr(
    not(debug_assertions),
    ignore = "NEOTMUX_PANIC_TEST hatch is compiled out in release"
)]
fn leaves_the_alternate_screen_on_panic() {
    let (out, ok, termios) = run_in_pty(&[("SHELL", "/bin/sh"), ("NEOTMUX_PANIC_TEST", "1")], b"");
    assert!(!ok, "expected a failing exit");
    assert!(
        out.contains("\x1b[?1049l"),
        "alternate screen was never left"
    );
    assert_termios_restored(&termios);
}

fn send_signal_and_capture(sig: Signal) -> (bool, Option<String>, Termios) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let isolation = tempfile::tempdir().expect("isolation dir");
    let _end_session = session::EndsSession::new(isolation.path());
    let log = isolation.path().join("neotmux.log");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("SHELL", "/bin/sh");
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
    cmd.env("XDG_RUNTIME_DIR", isolation.path());
    cmd.arg("--log");
    cmd.arg(&log);
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let collector = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        out
    });

    std::thread::sleep(Duration::from_millis(500));
    let pid = child.process_id().expect("pid");
    signal::kill(Pid::from_raw(pid as i32), sig).expect("kill");

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            break s;
        }
        assert!(
            Instant::now() < deadline,
            "neotmux did not exit after signal"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let termios = pair.master.get_termios().expect("termios after exit");
    drop(pair.master);
    collector.join().ok();
    (
        status.success(),
        status.signal().map(str::to_owned),
        termios,
    )
}

#[test]
fn restores_terminal_and_dies_of_sigterm() {
    let (ok, signal, termios) = send_signal_and_capture(Signal::SIGTERM);
    assert!(!ok, "expected neotmux to die of the signal, not exit 0");
    assert!(
        signal.is_some(),
        "expected the process to be reported as killed by a signal"
    );
    assert_termios_restored(&termios);
}

#[test]
fn restores_terminal_and_dies_of_sighup() {
    let (ok, signal, termios) = send_signal_and_capture(Signal::SIGHUP);
    assert!(!ok, "expected neotmux to die of the signal, not exit 0");
    assert!(
        signal.is_some(),
        "expected the process to be reported as killed by a signal"
    );
    assert_termios_restored(&termios);
}

struct TempScript(std::path::PathBuf);

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn temp_script_guard_removes_its_file_on_drop() {
    let path = std::env::temp_dir().join(format!("neotmux-guard-test-{}", std::process::id()));
    std::fs::write(&path, "x").expect("write");
    drop(TempScript(path.clone()));
    assert!(!path.exists(), "guard did not remove its file");
}

fn write_marker_shell(marker: &str) -> TempScript {
    use std::os::unix::fs::PermissionsExt as _;

    let path = std::env::temp_dir().join(format!("neotmux-marker-shell-{}", std::process::id()));
    std::fs::write(&path, format!("#!/bin/sh\nprintf '{marker}\\n'\nexit 0\n"))
        .expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    TempScript(path)
}

#[test]
fn renders_output_written_in_the_same_batch_as_exit() {
    let marker = "NEOTMUX_EOF_MARKER_4f2c";
    let shell = write_marker_shell(marker);
    let shell_path = shell.0.to_str().expect("utf8 path").to_owned();
    let (out, ok, _termios) = run_in_pty(&[("SHELL", &shell_path)], b"");
    assert!(ok, "expected a clean exit");
    assert!(
        out.contains(marker),
        "final PTY output written just before exit was never rendered: {out:?}"
    );
}

/// A child that asks its terminal what it is, waits for the answer, and reports whether one
/// came. Raw mode because the reply has no newline in it, and a generous read timeout so a
/// terminal that never answers is caught by the elapsed time rather than by the shell.
fn write_querying_shell() -> TempScript {
    use std::os::unix::fs::PermissionsExt as _;

    let path = std::env::temp_dir().join(format!("neotmux-query-shell-{}", std::process::id()));
    std::fs::write(
        &path,
        "#!/usr/bin/env bash\n\
         stty raw -echo\n\
         printf '\\033[c'\n\
         if IFS= read -r -s -d c -t 10 _reply; then\n\
         \tprintf 'DA1_ANSWERED\\r\\n'\n\
         else\n\
         \tprintf 'DA1_TIMEOUT\\r\\n'\n\
         fi\n",
    )
    .expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    TempScript(path)
}

/// The user's symptom, measured: nvim sends a run of queries and blocks on the reply to the
/// last of them, so against a terminal that never answers every `:q` waits out nvim's own
/// timeout — a whole second. The bound here is loose on purpose; what it separates is "the
/// terminal answered" from "the child gave up waiting".
#[test]
fn a_child_waiting_on_a_device_attributes_reply_is_not_left_waiting() {
    let shell = write_querying_shell();
    let shell_path = shell.0.to_str().expect("utf8 path").to_owned();

    let started = Instant::now();
    let (out, ok, _termios) = run_in_pty(&[("SHELL", &shell_path)], b"");
    let took = started.elapsed();

    assert!(ok, "expected a clean exit");
    assert!(
        out.contains("DA1_ANSWERED"),
        "the child never got an answer to CSI c: {out:?}"
    );
    assert!(
        took < Duration::from_secs(3),
        "a child that asked what its terminal was waited {took:?} for the answer"
    );
}

/// A PTY whose window size was never set reports 0x0, which is what `pty.fork()` and an
/// ssh session with no TTY size produce. Shell output there has to scroll a grid that
/// `vt100` cannot scroll unless the host clamps it up first.
#[test]
fn shell_output_on_a_zero_sized_terminal_does_not_kill_the_renderer() {
    let (_out, ok, _termios) = run_in_pty_sized(
        0,
        0,
        &[("SHELL", "/bin/sh")],
        b"printf 'aaaaaaaaaaaaaaaa\\n'\rexit\r",
    );
    assert!(ok, "neotmux died on a 0x0 terminal");
}

/// One below the measured floor on each axis, reached as a resize rather than at startup.
#[test]
fn resizing_below_the_floor_does_not_kill_the_renderer() {
    let (_out, ok, _termios) = run_in_pty_sized(
        1,
        1,
        &[("SHELL", "/bin/sh")],
        b"printf 'aaaaaaaaaaaaaaaa\\n'\rexit\r",
    );
    assert!(ok, "neotmux died on a 1x1 terminal");
}

/// A shell that closes its descriptors and keeps running, the way `setsid`, `ssh -f` and
/// anything else that daemonises does. It waits for a line first: a key only reaches the pane
/// once the client is attached and on the alternate screen, and a session that ends before
/// that never enters it, so there would be no `1049l` to look for.
fn write_daemonising_shell() -> TempScript {
    use std::os::unix::fs::PermissionsExt as _;

    let path = std::env::temp_dir().join(format!("neotmux-daemon-shell-{}", std::process::id()));
    std::fs::write(&path, "#!/bin/sh\nread _\nexec 0<&- 1>&- 2>&-\nsleep 10\n")
        .expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    TempScript(path)
}

/// EOF on the master says the descriptors are gone, not that the process is. Waiting on it
/// would hold the terminal in the alternate screen for as long as the child keeps running.
#[test]
fn shutdown_does_not_wait_on_a_child_that_outlived_its_descriptors() {
    let shell = write_daemonising_shell();
    let shell_path = shell.0.to_str().expect("utf8 path").to_owned();

    let started = Instant::now();
    let (out, ok, termios) = run_in_pty(&[("SHELL", &shell_path)], b"\r");
    let took = started.elapsed();

    assert!(ok, "expected a clean exit");
    assert!(
        took < Duration::from_secs(5),
        "shutdown waited {took:?} on a child that had only closed its descriptors"
    );
    assert!(
        out.contains("\x1b[?1049l"),
        "alternate screen was never left"
    );
    assert_termios_restored(&termios);
}

/// A neotmux whose terminal can be taken away mid-session, which is what a terminal emulator
/// being closed, or an ssh connection dropping, does to it. Nothing reads the master, so the
/// session is kept small enough not to fill the PTY's buffer before the terminal goes.
struct AbandonedTerminal {
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: i32,
    // Declared before `_isolation` so it drops first, while the directory it looks for a lock
    // in still exists. Held for that lifetime only — nothing reads it back.
    _end_session: session::EndsSession,
    _isolation: tempfile::TempDir,
}

impl AbandonedTerminal {
    fn start() -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let isolation = tempfile::tempdir().expect("isolation dir");
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
        cmd.env("TERM", "xterm-256color");
        cmd.env("SHELL", "/bin/cat");
        cmd.env("HOME", isolation.path());
        cmd.env("XDG_CONFIG_HOME", isolation.path());
        cmd.env("XDG_RUNTIME_DIR", isolation.path());
        cmd.arg("--log");
        cmd.arg(isolation.path().join("neotmux.log"));
        let child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);

        let pid = child.process_id().expect("pid") as i32;
        std::thread::sleep(Duration::from_millis(700));
        Self {
            master: Some(pair.master),
            child,
            pid,
            _end_session: session::EndsSession::new(isolation.path()),
            _isolation: isolation,
        }
    }

    /// Closes the last descriptor on the master, which is the whole of "the terminal is gone".
    fn abandon(&mut self) {
        self.master = None;
    }
}

impl Drop for AbandonedTerminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The CPU time the process itself has spent, in clock ticks, read from its own `utime` and
/// `stime`. A zombie still has both, so this can be sampled after it has exited and before
/// it is reaped — and it says nothing about how busy the runner is, which wall-clock would.
fn cpu_ticks(pid: i32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("read /proc stat");
    // The command name is parenthesised and may itself contain spaces.
    let rest = stat.rsplit_once(") ").expect("stat has a comm field").1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    utime + stime
}

/// The terminal going away is a hangup, and a hangup ends the session. Left undetected it is
/// not a hang but a spin: `crossterm::event::poll` answers a hung-up PTY by reading zero
/// bytes in a loop with no way out, so the process never returns to check its signal flags
/// and not even SIGTERM gets it back.
#[test]
fn exits_when_the_terminal_goes_away() {
    let mut session = AbandonedTerminal::start();
    session.abandon();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if session.child.try_wait().expect("try_wait").is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "neotmux was still running ten seconds after its terminal went away"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Exiting is only half of it: the four hours of pegged core this came from were spent
/// before anyone noticed. Measured on the process's own `utime + stime` rather than on how
/// long the wait took, so a loaded runner cannot fail it.
#[test]
fn burns_no_cpu_once_the_terminal_is_gone() {
    // A tenth of a core over the window. An idle session spends nothing; the spin spent all
    // of it.
    const TICK_BOUND: u64 = 20;

    let mut session = AbandonedTerminal::start();
    let before = cpu_ticks(session.pid);
    session.abandon();
    // Deliberately not reaped: the zombie keeps its /proc entry, and its CPU totals, alive.
    std::thread::sleep(Duration::from_secs(2));
    let after = cpu_ticks(session.pid);

    assert!(
        after - before <= TICK_BOUND,
        "neotmux spent {} ticks of CPU in the two seconds after its terminal went away",
        after - before
    );
}

/// Every frame is one synchronized update with the cursor hidden while its cells are written:
/// a pane pouring out a build otherwise showed half-painted frames and a cursor racing across
/// the screen, which is flicker.
#[test]
fn every_frame_is_one_synchronized_update_with_the_cursor_hidden() {
    let (out, ok, _termios) = run_in_pty(
        &[("SHELL", "/bin/sh")],
        b"for i in 1 2 3 4 5 6 7 8 9 10; do echo line-$i; done\rexit\r",
    );
    assert!(ok, "expected a clean exit");
    let begin = "\x1b[?2026h";
    let frames: Vec<&str> = out.split(begin).skip(1).collect();
    assert!(
        !frames.is_empty(),
        "no frame was a synchronized update: {out:?}"
    );
    for frame in &frames {
        assert!(
            frame.starts_with("\x1b[?25l"),
            "a frame wrote before hiding the cursor: {frame:?}"
        );
        assert!(
            frame.contains("\x1b[?2026l"),
            "a frame never ended its update: {frame:?}"
        );
    }
}

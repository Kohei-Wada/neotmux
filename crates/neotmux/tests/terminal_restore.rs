use std::io::Read;
use std::time::{Duration, Instant};

use nix::sys::signal::{self, Signal};
use nix::sys::termios::{LocalFlags, Termios};
use nix::unistd::Pid;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn run_in_pty(env: &[(&str, &str)], input: &[u8]) -> (String, bool, Termios) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let isolation = tempfile::tempdir().expect("isolation dir");
    let log = isolation.path().join("neotmux.log");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
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
    let log = isolation.path().join("neotmux.log");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("SHELL", "/bin/sh");
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
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

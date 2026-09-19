use std::io::Read;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn run_in_pty(env: &[(&str, &str)], input: &[u8]) -> (String, bool) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
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
    drop(pair.master);
    let out = collector.join().expect("collector");
    (String::from_utf8_lossy(&out).into_owned(), status.success())
}

#[test]
fn leaves_the_alternate_screen_on_clean_exit() {
    let (out, ok) = run_in_pty(&[("SHELL", "/bin/sh")], b"exit\r");
    assert!(ok, "expected a clean exit");
    assert!(
        out.contains("\x1b[?1049l"),
        "alternate screen was never left"
    );
}

#[test]
fn leaves_the_alternate_screen_on_panic() {
    let (out, ok) = run_in_pty(&[("SHELL", "/bin/sh"), ("NEOTMUX_PANIC_TEST", "1")], b"");
    assert!(!ok, "expected a failing exit");
    assert!(
        out.contains("\x1b[?1049l"),
        "alternate screen was never left"
    );
}

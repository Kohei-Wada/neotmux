use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const IDLE_BYTE_BOUND: usize = 64;

#[test]
fn produces_no_output_while_idle() {
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
    cmd.env("SHELL", "/bin/sh");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let (tx, rx) = mpsc::channel::<usize>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(n).is_err() {
                        break;
                    }
                }
            }
        }
    });

    std::thread::sleep(Duration::from_millis(700));
    while rx.try_recv().is_ok() {}

    let mut idle_bytes = 0usize;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(n) => idle_bytes += n,
            Err(_) => break,
        }
    }

    child.kill().ok();
    child.wait().ok();

    assert!(
        idle_bytes <= IDLE_BYTE_BOUND,
        "expected no redraw while nothing changed, got {idle_bytes} bytes"
    );
}

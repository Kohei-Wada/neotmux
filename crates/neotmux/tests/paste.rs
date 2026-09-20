//! A paste bigger than one read buffer. Everything past the first 1024 bytes used to sit in
//! the terminal until the next keystroke released another bufferful, and the process spun at
//! 100% against the bytes it was refusing to read. Both halves of that are asserted here.

use std::io::{Read, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

mod session;

/// Four kibibytes: four times the bufferful one readable edge used to hand over.
const PASTE: usize = 4096;

/// The process's own user + system time so far, in seconds. Fields 14 and 15 of
/// `/proc/<pid>/stat`, read after the command name so a shell called `a b c` cannot shift
/// them.
fn cpu_seconds(pid: u32) -> f64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("read /proc stat");
    let after_comm = stat.rsplit_once(')').expect("a comm field").1;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    let ticks: u64 =
        fields[11].parse::<u64>().expect("utime") + fields[12].parse::<u64>().expect("stime");
    ticks as f64 / 100.0
}

struct ChildGuard(Box<dyn Child + Send + Sync>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The pane's child: raw mode so the line discipline neither echoes the paste nor caps it at
/// one canonical line, then exactly `PASTE` bytes swallowed and a marker printed. The marker
/// is the assertion — it cannot be reached until the last pasted byte has arrived.
fn write_counting_shell(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("counting-shell");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nstty raw -echo\nprintf 'READY'\nhead -c {PASTE} >/dev/null\n\
             printf 'PASTE-COMPLETE'\nsleep 60\n"
        ),
    )
    .expect("write shell");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[test]
fn a_four_kibibyte_paste_arrives_whole_without_a_second_keystroke() {
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
    // An empty runtime layer: no status bar and no keymap, so every pasted byte falls
    // through to the pane rather than into a binding.
    let runtime = tempfile::tempdir().expect("runtime dir");
    let shell = write_counting_shell(isolation.path());

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("SHELL", &shell);
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
    cmd.env("XDG_RUNTIME_DIR", isolation.path());
    cmd.env("NEOTMUX_RUNTIME", runtime.path());
    cmd.arg("--log");
    cmd.arg(isolation.path().join("neotmux.log"));
    let child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);
    let pid = child.process_id().expect("a pid");
    let mut guard = ChildGuard(child);

    let screen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let seen = screen.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            seen.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });

    let shows = |marker: &str| screen.lock().unwrap().contains(marker);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !shows("READY") {
        assert!(Instant::now() < deadline, "the pane's child never started");
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut writer = pair.master.take_writer().expect("writer");
    let before = cpu_seconds(pid);
    writer.write_all(&vec![b'x'; PASTE]).expect("paste");
    writer.flush().expect("flush");

    // Deliberately nothing else is ever sent: the tail of a paste that needs a keystroke to
    // shake it loose is exactly the defect.
    const WINDOW: Duration = Duration::from_secs(1);
    std::thread::sleep(WINDOW);
    let spent = cpu_seconds(pid) - before;

    let complete = shows("PASTE-COMPLETE");
    drop(guard.0.kill());

    assert!(
        complete,
        "only part of a {PASTE}-byte paste reached the pane; the rest is still waiting for a \
         keystroke to release it"
    );
    assert!(
        spent <= 0.25,
        "the paste cost {spent:.3}s of CPU over {WINDOW:?}: the terminal is being polled \
         readable and not drained"
    );
}

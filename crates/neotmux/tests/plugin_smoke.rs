use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

// ratatui::init() requires a real tty, so this drives the binary through a PTY
// rather than with inherited stdio (see terminal_restore.rs's harness).
fn run_neotmux(runtime_dir: &Path, log: &Path) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let home = tempfile::tempdir().expect("isolated home");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.arg("--log");
    cmd.arg(log);
    cmd.env("NEOTMUX_RUNTIME", runtime_dir);
    cmd.env("SHELL", "/bin/true");
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", home.path());
    cmd.env("XDG_CONFIG_HOME", home.path());
    cmd.env("XDG_RUNTIME_DIR", home.path());
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let collector = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
            if n == 0 {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "neotmux did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(pair.master);
    collector.join().ok();
    assert!(status.success(), "neotmux exited with {status:?}");
}

fn read_log_containing(log: &Path, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = fs::read_to_string(log)
            && text.contains(marker)
        {
            return text;
        }
        assert!(Instant::now() < deadline, "marker never reached {log:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_plugin_loads_and_its_log_line_reaches_the_log_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lua/plugin")).unwrap();
    fs::write(dir.path().join("init.lua"), "require('plugin.hello')").unwrap();
    fs::write(
        dir.path().join("lua/plugin/hello.lua"),
        "neotmux.log.info('plugin-loaded-marker') return {}",
    )
    .unwrap();

    let log = dir.path().join("neotmux.log");
    run_neotmux(dir.path(), &log);

    let text = read_log_containing(&log, "plugin-loaded-marker");
    assert!(
        text.contains("plugin/hello"),
        "the log line is not attributed: {text}"
    );
}

#[test]
fn a_failed_plugin_is_warned_about_once() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lua/plugin")).unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "require('plugin.broken') require('plugin.hello')",
    )
    .unwrap();
    fs::write(
        dir.path().join("lua/plugin/broken.lua"),
        "error('deliberate-failure')",
    )
    .unwrap();
    fs::write(
        dir.path().join("lua/plugin/hello.lua"),
        "neotmux.log.info('plugin-loaded-marker') return {}",
    )
    .unwrap();

    let log = dir.path().join("neotmux.log");
    run_neotmux(dir.path(), &log);

    let text = read_log_containing(&log, "plugin-loaded-marker");

    let broken_lines: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("plugin/broken:"))
        .collect();
    assert_eq!(
        broken_lines.len(),
        1,
        "expected exactly one line about 'broken', got: {broken_lines:?}\nfull log: {text}"
    );
    assert!(
        broken_lines[0].contains("deliberate-failure"),
        "the one 'broken' line should be the load failure, got: {broken_lines:?}"
    );
}

#[test]
fn an_init_time_quit_exits_before_entering_the_alternate_screen() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lua/plugin")).unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "require('plugin.loud') neotmux.quit()",
    )
    .unwrap();
    fs::write(
        dir.path().join("lua/plugin/loud.lua"),
        "neotmux.log.info('loud-marker') return {}",
    )
    .unwrap();

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let log = dir.path().join("neotmux.log");
    let home = tempfile::tempdir().expect("isolated home");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.arg("--log");
    cmd.arg(&log);
    cmd.env("NEOTMUX_RUNTIME", dir.path());
    cmd.env("SHELL", "/bin/true");
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", home.path());
    cmd.env("XDG_CONFIG_HOME", home.path());
    cmd.env("XDG_RUNTIME_DIR", home.path());
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let collector = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        out
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "neotmux did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(pair.master);
    let out = collector.join().expect("collector");
    assert!(status.success(), "neotmux exited with {status:?}");
    assert!(
        !String::from_utf8_lossy(&out).contains("\x1b[?1049h"),
        "an init-time quit should exit before the alternate screen is entered"
    );

    let text = fs::read_to_string(&log).unwrap();
    assert!(
        text.contains("loud-marker"),
        "a load-time log line from before the quit should still reach the log: {text}"
    );
}

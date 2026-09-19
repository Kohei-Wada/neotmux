use std::fs;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

// ratatui::init() requires a real tty, so this drives the binary through a PTY
// rather than with inherited stdio (see terminal_restore.rs's harness).
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

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.arg("--log");
    cmd.arg(&log);
    cmd.env("NEOTMUX_RUNTIME", dir.path());
    cmd.env("SHELL", "/bin/true");
    cmd.env("TERM", "xterm-256color");
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

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = fs::read_to_string(&log)
            && text.contains("plugin-loaded-marker")
        {
            assert!(
                text.contains("hello"),
                "the log line is not attributed: {text}"
            );
            return;
        }
        assert!(Instant::now() < deadline, "marker never reached {log:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

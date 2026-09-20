use std::io::{Read, Write as _};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

struct ChildGuard(Box<dyn Child + Send + Sync>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct TempScript(std::path::PathBuf);

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_flooding_shell() -> TempScript {
    use std::os::unix::fs::PermissionsExt as _;

    let path = std::env::temp_dir().join(format!("neotmux-flood-shell-{}", std::process::id()));
    std::fs::write(&path, "#!/bin/sh\nexec yes\n").expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    TempScript(path)
}

#[test]
fn a_flooding_child_does_not_starve_ctrl_c() {
    let shell = write_flooding_shell();
    let shell_path = shell.0.to_str().expect("utf8 path").to_owned();

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
    cmd.env("SHELL", &shell_path);
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
    cmd.env("XDG_RUNTIME_DIR", isolation.path());
    cmd.arg("--log");
    cmd.arg(&log);
    let child = pair.slave.spawn_command(cmd).expect("spawn neotmux");
    drop(pair.slave);
    let mut guard = ChildGuard(child);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let drainer = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut writer = pair.master.take_writer().expect("writer");
        std::thread::sleep(Duration::from_secs(1));
        writer.write_all(&[0x03]).expect("write ctrl-c");
        writer.flush().expect("flush");
        drop(writer);

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = guard.0.try_wait().expect("try_wait") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "neotmux did not exit after ctrl-c reached a flooding child"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }));

    drop(guard);
    drop(pair.master);
    drainer.join().ok();

    match result {
        Ok(status) => assert!(status.success(), "expected neotmux to exit cleanly"),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

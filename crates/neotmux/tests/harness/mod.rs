use std::io::{Read, Write as _};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// The env/args every neotmux test invocation needs: an isolated HOME and
/// XDG_CONFIG_HOME (so the binary never touches the developer's own config)
/// and a log file inside that same isolation directory.
pub fn base_command(log: &Path, isolation: &Path) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", isolation);
    cmd.env("XDG_CONFIG_HOME", isolation);
    cmd.arg("--log");
    cmd.arg(log);
    cmd
}

/// Writes a small plugin runtime layout (`init.lua`, `lua/plugin/*.lua`, ...)
/// to a fresh temp dir, suitable for `NEOTMUX_RUNTIME`.
pub fn runtime(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("runtime dir");
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create runtime subdir");
        }
        std::fs::write(&full, content).expect("write runtime file");
    }
    dir
}

const ROWS: u16 = 24;
const COLS: u16 = 80;

/// portable-pty's SHELL resolution requires an executable path (it calls
/// `access(2)`, not a `$PATH` search), so a bare name like "cat" needs
/// resolving here before it is handed down as `$SHELL`.
fn resolve_shell(shell: &str) -> String {
    if shell.contains('/') {
        return shell.to_string();
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let candidate = Path::new(dir).join(shell);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    shell.to_string()
}

/// A live neotmux process driven over a real PTY, with its output rendered
/// into a `vt100::Parser` so tests can assert on the screen rather than on
/// raw escape bytes.
pub struct Neotmux {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn std::io::Write + Send>,
    master: Box<dyn MasterPty + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    byte_rx: mpsc::Receiver<usize>,
    _isolation: tempfile::TempDir,
}

impl Neotmux {
    pub fn start(dir: &tempfile::TempDir) -> Self {
        Self::start_with_shell(dir, "/bin/cat")
    }

    pub fn start_with_shell(dir: &tempfile::TempDir, shell: &str) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let isolation = tempfile::tempdir().expect("isolation dir");
        let log = isolation.path().join("neotmux.log");

        let mut cmd = base_command(&log, isolation.path());
        cmd.env("NEOTMUX_RUNTIME", dir.path());
        cmd.env("SHELL", resolve_shell(shell));

        let child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let writer = pair.master.take_writer().expect("take writer");
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
        let (tx, byte_rx) = mpsc::channel();

        let parser_clone = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        parser_clone.lock().unwrap().process(&buf[..n]);
                        if tx.send(n).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            child,
            writer,
            master: pair.master,
            parser,
            byte_rx,
            _isolation: isolation,
        }
    }

    pub fn send(&mut self, s: &str) {
        self.writer.write_all(s.as_bytes()).expect("write to pty");
        self.writer.flush().expect("flush pty");
    }

    fn screen_contents(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    pub fn wait_for_screen(&mut self, marker: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.screen_contents();
            if screen.contains(marker) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "marker {marker:?} never appeared on screen:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn screen_after(&mut self, duration: Duration) -> String {
        std::thread::sleep(duration);
        self.screen_contents()
    }

    pub fn settle(&mut self, duration: Duration) {
        std::thread::sleep(duration);
        while self.byte_rx.try_recv().is_ok() {}
    }

    pub fn bytes_in(&mut self, duration: Duration) -> usize {
        std::thread::sleep(duration);
        let mut total = 0usize;
        while let Ok(n) = self.byte_rx.try_recv() {
            total += n;
        }
        total
    }
}

impl Drop for Neotmux {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = &self.master;
    }
}

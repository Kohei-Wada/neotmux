use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// The env/args every neotmux test invocation needs: an isolated HOME and
/// XDG_CONFIG_HOME (so the binary never touches the developer's own config)
/// and a log file inside that same isolation directory.
pub fn base_command(exe: &Path, log: &Path, isolation: &Path) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(exe);
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

/// The checked-in fixture plugin set (§4.11): an ordinary runtime layer of `.lua` files,
/// stacked behind a layer that supplies the `init.lua` choosing what to require.
pub fn fixture_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
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
    log: PathBuf,
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
        Self::start_layered(&[dir.path()], shell)
    }

    pub fn start_layered(layers: &[&Path], shell: &str) -> Self {
        Self::start_sized(layers, shell, ROWS, COLS)
    }

    /// A terminal of a chosen size, for the cases where the size is the point — a surface
    /// too small to split, for instance.
    pub fn start_sized(layers: &[&Path], shell: &str, rows: u16, cols: u16) -> Self {
        let runtime = layers
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":");
        Self::spawn(
            env!("CARGO_BIN_EXE_neotmux").as_ref(),
            Some(runtime),
            shell,
            rows,
            cols,
        )
    }

    /// An installed tree: `<prefix>/bin/neotmux` run with no `NEOTMUX_RUNTIME`, so the
    /// shipped layer has to be found the way an installed neotmux finds it — next to the
    /// binary. Pointing the tests at the source tree instead is what let an install ship a
    /// layer that errored on every start.
    pub fn start_installed(prefix: &Path, shell: &str) -> Self {
        Self::spawn(&prefix.join("bin/neotmux"), None, shell, ROWS, COLS)
    }

    fn spawn(exe: &Path, runtime: Option<String>, shell: &str, rows: u16, cols: u16) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let isolation = tempfile::tempdir().expect("isolation dir");
        let log = isolation.path().join("neotmux.log");

        let mut cmd = base_command(exe, &log, isolation.path());
        if let Some(runtime) = runtime {
            cmd.env("NEOTMUX_RUNTIME", runtime);
        }
        cmd.env("SHELL", resolve_shell(shell));

        let child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let writer = pair.master.take_writer().expect("take writer");
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
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
            log,
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

    /// One cell of the reconstructed screen, so a test can say not just that something is on
    /// screen but which column it is in. Empty for a cell the screen does not have.
    pub fn cell(&self, row: u16, col: u16) -> String {
        self.parser
            .lock()
            .unwrap()
            .screen()
            .cell(row, col)
            .map(|cell| cell.contents().to_string())
            .unwrap_or_default()
    }

    /// The row the shipped status bar reserves for itself, as text.
    pub fn status(&self) -> String {
        self.screen_contents()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    }

    /// Blocks until the status bar says `marker`. What the bar says is the assertion in most
    /// of these tests, and it is also how they know a split or a switch has landed.
    pub fn wait_for_status(&mut self, marker: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let status = self.status();
            if status.contains(marker) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the status bar never said {marker:?}; it says {status:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Blocks until one cell holds `expected` — a pane boundary appearing, most often.
    pub fn wait_for_cell(&mut self, row: u16, col: u16, expected: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if self.cell(row, col) == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "cell ({row}, {col}) never held {expected:?}:\n{}",
                self.screen_contents()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
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

    /// Blocks until `marker` appears in the isolated log file, then returns the whole log.
    pub fn wait_for_log(&self, marker: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let text = std::fs::read_to_string(&self.log).unwrap_or_default();
            if text.contains(marker) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "marker {marker:?} never reached the log:\n{text}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Blocks until the process is gone, and reports whether it left cleanly.
    pub fn exited_cleanly(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status.success();
            }
            assert!(Instant::now() < deadline, "neotmux did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A byte in, the first byte of the redraw out — the round trip a hand feels as
    /// latency. `None` if nothing came back inside `timeout`. The screen is let go quiet for
    /// `quiet` first, so what comes back belongs to this keystroke and not to an earlier one;
    /// the caller varies that so a run of samples does not land at the same point of the
    /// frame every time.
    pub fn round_trip(
        &mut self,
        keys: &str,
        quiet: Duration,
        timeout: Duration,
    ) -> Option<Duration> {
        self.settle(quiet);
        let started = Instant::now();
        self.send(keys);
        self.byte_rx
            .recv_timeout(timeout)
            .ok()
            .map(|_| started.elapsed())
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

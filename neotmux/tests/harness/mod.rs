//! The harness is a fixture library shared by every test crate that starts a client, and
//! each crate uses a different slice of it, so an item dead in one crate is live in
//! another and the allow is structural, not a waiver.
#![allow(dead_code)]

use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::session;

/// The env/args every neotmux test invocation needs: an isolated HOME,
/// XDG_CONFIG_HOME and XDG_RUNTIME_DIR (so the binary never touches the
/// developer's own config, and never shares a session with the test running
/// beside it) and a log file inside that same isolation directory.
pub fn base_command(exe: &Path, log: &Path, isolation: &Path) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(exe);
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", isolation);
    cmd.env("XDG_CONFIG_HOME", isolation);
    cmd.env("XDG_RUNTIME_DIR", isolation);
    cmd.arg("--log");
    cmd.arg(log);
    cmd
}

/// What every runtime this harness writes turns off before the test's own `init.lua` runs.
///
/// These are real servers with the real job runner, and `chime` is on by default: any pane a
/// fixture drives into `blocked` or out of `working` spawns a player and `cargo test` makes a
/// noise at whoever ran it. A test about the chime turns it back on for itself.
///
/// Under a `pcall` because not every test stacks the shipped layer behind this one, and where
/// it is absent there is no `neotmux.defaults` to reach and nothing to silence either.
///
/// The tick budget is loosened too. It is measured on the wall clock, so on a loaded runner the
/// time a tick spends waiting for a CPU counts against it, and an unoptimised build has little
/// room under 8 ms: a shipped plugin overruns, logs an error or is unsubscribed, and a test
/// about something else fails on it. A test about the budget sets its own.
const QUIET: &str = "pcall(function() require('neotmux.defaults').chime = false end)\n\
                     neotmux.opt.tick_budget_ms = 250\n";

/// What these tests wait on, appended to every `init.lua` this harness writes: on the right of
/// the shipped bar, the window's position, the focused pane's after a dot (and `Z` while it is
/// zoomed), then what it is running and where — `1.2  cat` once a split has landed and focused
/// the new pane. herdr's bar says none of it, so it is added the way a user adds to the right,
/// and only where the test's own `init.lua` left the bar on.
const PANE_MARKER: &str = "pcall(function()\n\
       local defaults = require('neotmux.defaults')\n\
       if not (defaults.tmux and defaults.status) then return end\n\
       require('plugin.tmux.status').right = { function()\n\
         local window, pane = neotmux.window.current(), neotmux.pane.current()\n\
         local function at(list, id)\n\
           for i, it in ipairs(list or {}) do if it == id then return i end end\n\
         end\n\
         local w, p = at(neotmux.window.list(), window), at(neotmux.window.panes(window), pane)\n\
         if not (w and p) then return end\n\
         local info = neotmux.pane.info(pane) or {}\n\
         return w .. '.' .. p .. (neotmux.pane.zoomed() and 'Z' or '') .. '  '\n\
           .. (info.command or '') .. '  ' .. (info.cwd or '')\n\
       end }\n\
     end)\n";

/// Writes a small plugin runtime layout (`init.lua`, `lua/plugin/*.lua`, ...)
/// to a fresh temp dir, suitable for `NEOTMUX_RUNTIME`.
pub fn runtime(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("runtime dir");
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create runtime subdir");
        }
        let content = match *path {
            "init.lua" => format!("{QUIET}{content}\n{PANE_MARKER}"),
            _ => content.to_string(),
        };
        std::fs::write(&full, content).expect("write runtime file");
    }
    dir
}

/// The checked-in fixture plugin set (§4.11): an ordinary runtime layer of `.lua` files,
/// stacked behind a layer that supplies the `init.lua` choosing what to require.
pub fn fixture_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub const ROWS: u16 = 24;
pub const COLS: u16 = 80;

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
    // Declared before `_dir_keep` so it drops first: fields drop in declaration order, and
    // ending the session needs the isolation dir to still be there.
    _end_session: session::EndsSession,
    isolation: PathBuf,
    // Owns the isolation dir when this harness made it (`spawn`); `None` when a test lent us
    // one it keeps alive (`start_named_on`). Declared after `_end_session` so the dir is only
    // released once the session behind it is already ended.
    _dir_keep: Option<tempfile::TempDir>,
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
            None,
        )
    }

    /// Started from a directory of the caller's choosing — the point of the tests that
    /// ask where a pane begins. `layers` are the runtime layers, as before.
    pub fn start_layered_in(layers: &[&Path], shell: &str, cwd: &Path) -> Self {
        Self::start_sized_in(layers, shell, ROWS, COLS, cwd)
    }

    pub fn start_sized_in(layers: &[&Path], shell: &str, rows: u16, cols: u16, cwd: &Path) -> Self {
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
            Some(cwd),
        )
    }

    /// An installed tree: `<prefix>/bin/neotmux` run with no `NEOTMUX_RUNTIME`, so the
    /// shipped layer has to be found the way an installed neotmux finds it — next to the
    /// binary. Pointing the tests at the source tree instead is what let an install ship a
    /// layer that errored on every start.
    pub fn start_installed(prefix: &Path, shell: &str) -> Self {
        Self::spawn(&prefix.join("bin/neotmux"), None, shell, ROWS, COLS, None)
    }

    fn spawn(
        exe: &Path,
        runtime: Option<String>,
        shell: &str,
        rows: u16,
        cols: u16,
        cwd: Option<&Path>,
    ) -> Self {
        let isolation = tempfile::tempdir().expect("isolation dir");
        let log = isolation.path().join("neotmux.log");

        let mut cmd = base_command(exe, &log, isolation.path());
        if let Some(runtime) = runtime {
            cmd.env("NEOTMUX_RUNTIME", runtime);
        }
        // The client's directory is the server's: the server is this client's grandchild
        // with nothing between them that touches `chdir`, so `env::current_dir` in the
        // server is the directory the test says neotmux was started from.
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }
        cmd.env("SHELL", resolve_shell(shell));

        // We made the isolation dir, so we are the one who must keep it alive past the
        // client's drop: `spawn_with` holds it in `_dir_keep` for exactly that.
        let isolation_path = isolation.path().to_owned();
        Self::spawn_with(cmd, &isolation_path, &log, Some(isolation), rows, cols)
    }

    /// The tail of starting a client, with the command and the isolation dir handed in:
    /// opens the pty, spawns it, wires the reader thread and the parser, and builds the
    /// `Neotmux`. `spawn` (we made the dir) and `start_named_on` (a test lent it to us and
    /// keeps it alive) both end here; `keep` is `Some` in the first case and `None` in the
    /// second, and it is what lets a dir we own outlive the client it is holding.
    fn spawn_with(
        cmd: CommandBuilder,
        isolation: &Path,
        log: &Path,
        keep: Option<tempfile::TempDir>,
        rows: u16,
        cols: u16,
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

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

        let nt = Self {
            log: log.to_owned(),
            child,
            writer,
            master: pair.master,
            parser,
            byte_rx,
            _end_session: session::EndsSession::new(isolation),
            isolation: isolation.to_owned(),
            _dir_keep: keep,
        };
        nt.wait_until_raw();
        nt
    }

    /// Blocks until neotmux has taken the terminal, and panics rather than let a test run
    /// against a terminal it does not own yet.
    ///
    /// Until then the line discipline answers the keyboard: it eats `C-w` as WERASE, turns
    /// `C-c` into SIGINT, rewrites `\r` as `\n`, and echoes whatever is written — so a test
    /// that waits for its own input to appear on screen is watching the kernel, not neotmux,
    /// and the next key it sends can still be swallowed. Only neotmux can clear `ICANON`, so
    /// that is the barrier. It also strengthens every `wait_for_screen`: raw mode turns the
    /// echo off, so afterwards a byte on screen got there by neotmux reading it.
    fn wait_until_raw(&self) {
        let Some(fd) = self.master.as_raw_fd() else {
            panic!("no pty fd to read the line discipline from");
        };
        let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(termios) = nix::sys::termios::tcgetattr(fd)
                && !termios
                    .local_flags
                    .contains(nix::sys::termios::LocalFlags::ICANON)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("neotmux never took the terminal out of canonical mode");
    }

    /// Resizes the outer terminal. The kernel announces this to neotmux as `SIGWINCH` and in
    /// no other way — nothing is sent on the terminal to say so — so this is the only way a
    /// test can exercise the path a real resize takes.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize pty");
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(rows, cols);
    }

    /// The client's `$XDG_RUNTIME_DIR`, which is where the CLI has to be pointed to reach the
    /// session this client is attached to.
    pub fn isolation(&self) -> &Path {
        &self.isolation
    }

    pub fn send(&mut self, s: &str) {
        self.writer.write_all(s.as_bytes()).expect("write to pty");
        self.writer.flush().expect("flush pty");
    }

    pub fn screen_contents(&self) -> String {
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

    /// One cell's background, as it reached the outer terminal. A tint is a colour and
    /// nothing else — the glyph it leaves alone is what `cell` already answers — so this is
    /// the only way a PTY test can see one at all. `None` for a cell left at the terminal's
    /// own background or set from the palette rather than by name.
    /// A cell's foreground, as an indexed colour. What a test asks when the question is
    /// whether the pane's own colours survived something.
    pub fn cell_fg_idx(&self, row: u16, col: u16) -> Option<u8> {
        match self
            .parser
            .lock()
            .unwrap()
            .screen()
            .cell(row, col)?
            .fgcolor()
        {
            vt100::Color::Idx(i) => Some(i),
            _ => None,
        }
    }

    pub fn cell_bg(&self, row: u16, col: u16) -> Option<(u8, u8, u8)> {
        match self
            .parser
            .lock()
            .unwrap()
            .screen()
            .cell(row, col)?
            .bgcolor()
        {
            vt100::Color::Rgb(r, g, b) => Some((r, g, b)),
            _ => None,
        }
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

    /// Blocks until `ready` holds, for what no `wait_for_*` names — a colour, a count, a
    /// marker going away — and fails with `what` and the screen if it never does.
    pub fn wait_until(&self, what: &str, timeout: Duration, ready: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + timeout;
        while !ready(self) {
            assert!(
                Instant::now() < deadline,
                "{what}:\n{}",
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

    /// Ends this client and waits for it to exit, the way `socket.rs`'s `Client::kill`
    /// does, on the harness side, because the tests that kill a client are the ones
    /// `sessions.rs` and `two-clients` run.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        self.exited_cleanly(Duration::from_secs(10));
    }

    /// A client into a runtime directory the test holds: `ls` is a connect-probe of that
    /// directory and `kill [name]` signals into it, so the test has to point at the same
    /// one the client was using, and the one that is not its own is this. The test owns
    /// the directory and keeps it alive; this borrows it.
    pub fn start_named_on(
        runtime: &Path,
        session: &str,
        layer: &tempfile::TempDir,
        shell: &str,
    ) -> Self {
        let exe = Path::new(env!("CARGO_BIN_EXE_neotmux"));
        let log = runtime.join(format!("neotmux-{session}.log"));
        let mut cmd = base_command(exe, &log, runtime);
        cmd.env(
            "NEOTMUX_RUNTIME",
            layer.path().to_string_lossy().into_owned(),
        );
        cmd.arg(session);
        cmd.env("SHELL", resolve_shell(shell));
        // The test owns `runtime` and keeps it alive after this client is gone, so there
        // is nothing for us to keep: `keep` is `None`.
        Self::spawn_with(cmd, runtime, &log, None, ROWS, COLS)
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
                "marker {marker:?} never reached the log:\n{text}\n{}",
                self.threads()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// What every thread of the neotmux process is doing, as the kernel sees it: its state
    /// letter and the function it is sleeping in. A `wait_for_log` that times out on a
    /// runner nobody can reproduce on is otherwise a bare "it never said that" — this says
    /// whether the loop is blocked in a read, spinning, or gone.
    fn threads(&self) -> String {
        let Some(pid) = self.child.process_id() else {
            return "the process is already gone".to_string();
        };
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
            return format!("no /proc/{pid}/task to read");
        };
        let mut report = format!("threads of neotmux ({pid}):");
        for task in tasks.flatten() {
            let dir = task.path();
            let read = |name: &str| {
                std::fs::read_to_string(dir.join(name))
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            let stat = read("stat");
            let state = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_string))
                .unwrap_or_default();
            report.push_str(&format!(
                "\n  {} {} state={state} wchan={}",
                task.file_name().to_string_lossy(),
                read("comm"),
                read("wchan"),
            ));
        }
        report
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Blocks until the process is gone, and reports whether it left cleanly.
    /// How the client left, once it has — whatever it left with.
    ///
    /// A session ended out from under a client is not a clean exit: the client says the
    /// server went away and exits non-zero, which is the right thing to say and the wrong
    /// thing to assert `success()` about. What a kill is asked here is that the client is
    /// out, not how it took it.
    pub fn wait_for_exit(&mut self, timeout: Duration) -> Option<portable_pty::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

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
    }
}

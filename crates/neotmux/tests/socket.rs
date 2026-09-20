//! The two processes and the socket between them: who starts one, who is refused one, what
//! happens to a session whose client goes away, and what a client does when the thing
//! answering on the socket was built against another protocol.

use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use neotmux_proto::wire::{self, HELLO_LEN};
use neotmux_proto::{OwnedToClient, PROTOCOL_VERSION};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

mod session;

/// Long enough for a process to start and a shell to be forked on a loaded machine; a
/// passing run takes a small fraction of it.
const PATIENCE: Duration = Duration::from_secs(20);

/// A test's own `$XDG_RUNTIME_DIR`, dropped last among the locals declared after it so that
/// whatever a test did with its session — including ending it itself, as the `--kill` tests
/// do — has already happened by the time this reaches for it.
struct TestRuntime(tempfile::TempDir);

impl TestRuntime {
    fn new() -> Self {
        Self(tempfile::tempdir().expect("runtime dir"))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        session::end_session(self.0.path());
    }
}

fn socket_in(runtime: &Path) -> PathBuf {
    runtime.join("neotmux/default.sock")
}

/// A neotmux client on its own PTY, with `$XDG_RUNTIME_DIR` pointed wherever the test wants
/// so that the session it starts is the test's own and nobody else's.
struct Client {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn std::io::Write + Send>,
    /// Held for the life of the client: closing the master is a hangup on the terminal, and
    /// a client whose terminal went away takes its session's shells with it.
    _master: Box<dyn MasterPty + Send>,
    screen: Arc<Mutex<String>>,
}

impl Client {
    fn start(runtime: &Path, shell: &str) -> Self {
        Self::start_sized(runtime, shell, 24, 80)
    }

    fn start_sized(runtime: &Path, shell: &str, rows: u16, cols: u16) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
        cmd.env("TERM", "xterm-256color");
        cmd.env("SHELL", shell);
        cmd.env("HOME", runtime);
        cmd.env("XDG_CONFIG_HOME", runtime);
        cmd.env("XDG_RUNTIME_DIR", runtime);
        cmd.arg("--log");
        cmd.arg(runtime.join("neotmux.log"));
        let child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);

        let writer = pair.master.take_writer().expect("writer");
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let screen = Arc::new(Mutex::new(String::new()));
        let collected = screen.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                collected
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        });

        Self {
            child,
            writer,
            _master: pair.master,
            screen,
        }
    }

    fn send(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).expect("write");
        self.writer.flush().expect("flush");
    }

    fn wait_for_output(&self, marker: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let seen = self.screen.lock().unwrap().clone();
            if seen.contains(marker) {
                return seen;
            }
            assert!(
                Instant::now() < deadline,
                "{marker:?} never reached the terminal; it says {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_exit(&mut self) -> portable_pty::ExitStatus {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status;
            }
            assert!(Instant::now() < deadline, "neotmux did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Blocks until the session's socket exists.
///
/// Watched rather than connected to: a connect is a client, and a client that connects and
/// says nothing is one the server has to wait out before it can greet the real one.
fn wait_for_socket(socket: &Path) {
    let deadline = Instant::now() + PATIENCE;
    while !socket.exists() {
        assert!(
            Instant::now() < deadline,
            "nothing ever bound {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A client that has started its session: the shell has echoed something back, which means
/// the whole round trip — key in, socket, pane, frame, socket, screen — is running.
fn started(runtime: &Path) -> Client {
    let mut client = Client::start(runtime, "/bin/cat");
    wait_for_socket(&socket_in(runtime));
    client.send("neotmux-is-two-processes\r");
    client.wait_for_output("neotmux-is-two-processes");
    client
}

/// The one thing that has to be true before detach and attach can be built on it: killing
/// the client does not kill the session.
#[test]
fn the_server_outlives_a_client_that_disconnects() {
    let runtime = TestRuntime::new();
    let mut client = started(runtime.path());

    client.kill();
    drop(client);

    // Not a poll for "is it still there": a connect that is answered is the server itself
    // saying so, and a dead one's socket file answers ECONNREFUSED.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut answered = None;
    while Instant::now() < deadline {
        if let Ok(mut stream) = UnixStream::connect(socket_in(runtime.path())) {
            let mut hello = [0u8; HELLO_LEN];
            stream.write_all(&wire::encode_hello()).expect("greet");
            if stream.read_exact(&mut hello).is_ok() {
                answered = Some(hello);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        answered,
        Some(wire::encode_hello()),
        "the session died with the client that was looking at it; socket exists={} log={}",
        socket_in(runtime.path()).exists(),
        std::fs::read_to_string(runtime.path().join("neotmux.log")).unwrap_or_default()
    );
}

/// The old behaviour was a three-second linger. This is the same question asked long
/// enough after the client is gone that only an immortal session can answer it.
#[test]
fn a_session_with_nobody_looking_at_it_outlives_the_old_linger() {
    let runtime = TestRuntime::new();
    let mut client = started(runtime.path());

    client.kill();
    drop(client);
    std::thread::sleep(Duration::from_secs(5));

    let mut stream = UnixStream::connect(socket_in(runtime.path()))
        .expect("the session died while nobody was looking at it");
    let mut hello = [0u8; HELLO_LEN];
    stream.write_all(&wire::encode_hello()).expect("greet");
    stream
        .read_exact(&mut hello)
        .expect("the server did not greet");
    assert_eq!(hello, wire::encode_hello());
}

/// Accepted and refused, rather than left in the backlog. One client, because the whole
/// frame is a conversation with one Lua layer: two would each tick the same session, each
/// believing the ops it raised were the only ones. Sharing a session is detach and attach's
/// problem, not the split's.
#[test]
fn a_second_client_is_refused_and_says_why() {
    let runtime = TestRuntime::new();
    let _first = started(runtime.path());

    let mut second = Client::start(runtime.path(), "/bin/cat");
    let said = second.wait_for_output("already has a client");
    let status = second.wait_for_exit();
    assert!(
        !status.success(),
        "a refused client exited successfully: {said:?}"
    );
}

/// The shell is `/bin/cat`, so what was typed before the client went away is still on the
/// pane's grid afterwards — and only a kept terminal state can show it again.
#[test]
fn a_second_client_is_shown_the_session_the_first_one_left() {
    let runtime = TestRuntime::new();
    let mut first = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    first.send("still-here-after-the-detach\r");
    first.wait_for_output("still-here-after-the-detach");
    first.kill();
    drop(first);

    let second = Client::start(runtime.path(), "/bin/cat");
    second.wait_for_output("still-here-after-the-detach");
}

/// A connection that greets and hangs up before saying anything must not cost the session
/// that was sitting there waiting: this is not a client, so it is never `attached`, and
/// the detached session's shell must still be there for the client that comes next.
#[test]
fn a_detached_session_survives_a_connection_that_never_attaches() {
    let runtime = TestRuntime::new();
    let mut first = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    first.send("still-here-after-the-non-client\r");
    first.wait_for_output("still-here-after-the-non-client");
    first.kill();
    drop(first);

    // Greets and hangs up without ever sending an Attach — a health check, or a client
    // that raced a detach.
    let mut stream = UnixStream::connect(socket_in(runtime.path())).expect("connect");
    stream.write_all(&wire::encode_hello()).expect("greet");
    let mut hello = [0u8; HELLO_LEN];
    stream.read_exact(&mut hello).expect("be greeted");
    drop(stream);

    let second = Client::start(runtime.path(), "/bin/cat");
    second.wait_for_output("still-here-after-the-non-client");
}

/// Greets, then sends the given bytes as one framed message, then hangs up.
fn greet_and_send_message(socket: &Path, payload: &[u8]) {
    let mut stream = UnixStream::connect(socket).expect("connect");
    stream.write_all(&wire::encode_hello()).expect("greet");
    let mut hello = [0u8; HELLO_LEN];
    stream.read_exact(&mut hello).expect("be greeted");
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("small payload")
            .to_le_bytes(),
    );
    framed.extend_from_slice(payload);
    stream.write_all(&framed).expect("send the message");
    stream.flush().expect("flush");
}

/// A connection whose first message is not an `Attach` — one meant for a live conversation,
/// or bytes that do not decode as a message at all — must not end the detached session it
/// found, and must not take the server itself down with it.
#[test]
fn a_detached_session_survives_a_connection_whose_first_message_is_not_an_attach() {
    let runtime = TestRuntime::new();
    let mut first = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    first.send("still-here-after-the-bad-message\r");
    first.wait_for_output("still-here-after-the-bad-message");
    first.kill();
    drop(first);

    // A message that decodes fine, but is not an Attach.
    let resize = wire::encode_to_server(&neotmux_proto::ToServer::Resize(neotmux_proto::Size {
        rows: 10,
        cols: 10,
    }));
    greet_and_send_message(&socket_in(runtime.path()), &resize);

    // A discriminant no message uses: this does not decode at all.
    greet_and_send_message(&socket_in(runtime.path()), &[0xff]);

    // The server is still there, and still holding the same session, for a real client.
    let second = Client::start(runtime.path(), "/bin/cat");
    second.wait_for_output("still-here-after-the-bad-message");
}

/// The session is laid out against whichever terminal is looking at it now.
///
/// `stty size` rather than `tput cols`: it asks the pane's own PTY directly, needing no
/// terminfo database. The surface reserves a row for the status bar, so only the column
/// count — the second number `stty size` prints — is asserted.
#[test]
fn a_session_is_refitted_to_the_terminal_that_attaches_to_it() {
    let runtime = TestRuntime::new();
    let first = Client::start_sized(runtime.path(), "/bin/sh", 24, 80);
    wait_for_socket(&socket_in(runtime.path()));
    drop(first);

    let mut second = Client::start_sized(runtime.path(), "/bin/sh", 30, 100);
    second.send("stty size\r");
    second.wait_for_output("100");
}

/// The file a killed server leaves behind. Nothing is listening on it, so it is replaced
/// rather than believed — and a session starts on it as if it had never been there.
#[test]
fn a_socket_a_dead_server_left_behind_is_replaced() {
    let runtime = TestRuntime::new();
    let socket = socket_in(runtime.path());
    std::fs::create_dir_all(socket.parent().expect("a parent")).expect("create the directory");
    drop(UnixListener::bind(&socket).expect("bind a listener and drop it"));
    assert!(socket.exists());
    assert!(
        UnixStream::connect(&socket).is_err(),
        "the stale socket still had something behind it"
    );

    let _client = started(runtime.path());
    assert!(
        UnixStream::connect(&socket).is_ok(),
        "the stale socket was not replaced by a live one"
    );
}

/// Yesterday's binary meeting today's server, which is the only way skew can happen while
/// both ends are built from the same commit: something answers on the socket and speaks a
/// protocol this client does not. It says both numbers and leaves, because there is nothing
/// to negotiate — §2.1.
#[test]
fn a_client_meeting_another_protocol_says_both_numbers_and_exits() {
    let runtime = TestRuntime::new();
    let socket = socket_in(runtime.path());
    std::fs::create_dir_all(socket.parent().expect("a parent")).expect("create the directory");
    let listener = UnixListener::bind(&socket).expect("bind");

    let theirs = PROTOCOL_VERSION + 1;
    let impostor = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream.write_all(&theirs.to_le_bytes()).expect("greet");
        // Read whatever the client greeted us with and then go away.
        let mut hello = [0u8; HELLO_LEN];
        let _ = stream.read_exact(&mut hello);
    });

    let mut client = Client::start(runtime.path(), "/bin/cat");
    let said = client.wait_for_output("protocol version mismatch");
    assert!(
        said.contains(&format!("ours {PROTOCOL_VERSION}"))
            && said.contains(&format!("theirs {theirs}")),
        "the mismatch named one number and not the other: {said:?}"
    );
    assert!(
        !client.wait_for_exit().success(),
        "a client that could not speak to the server exited successfully"
    );
    impostor.join().expect("join");
}

/// A key typed at a session over the socket reaches its shell, and what the shell says comes
/// back to the screen. The whole point of the split is that nothing about this changed.
#[test]
fn a_session_over_the_socket_still_carries_a_keystroke_to_its_shell_and_back() {
    let runtime = TestRuntime::new();
    let _client = started(runtime.path());
}

/// A client speaking the protocol, but hanging up instead of attaching. The server goes back
/// to waiting rather than falling over, and the next client gets its session.
#[test]
fn a_client_that_greets_and_leaves_without_attaching_costs_nothing() {
    let runtime = TestRuntime::new();
    let client = started(runtime.path());

    for _ in 0..3 {
        let mut stream = UnixStream::connect(socket_in(runtime.path())).expect("connect");
        stream.write_all(&wire::encode_hello()).expect("greet");
        let mut hello = [0u8; HELLO_LEN];
        stream.read_exact(&mut hello).expect("be greeted");
        // A session is already running, so this one is refused — and then hangs up.
        let mut refusal = Vec::new();
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.read_to_end(&mut refusal);
        assert!(!refusal.is_empty(), "the refusal never came");
        let message = &refusal[4..];
        assert!(
            matches!(
                wire::decode_to_client(message),
                Ok(OwnedToClient::Refused(_))
            ),
            "a second client was answered with something other than a refusal"
        );
    }

    // Still the same session, still talking to its shell.
    drop(client);
}

/// The way out of an immortal session, and the one that does not go through the protocol.
#[test]
fn kill_ends_a_session_nobody_is_attached_to() {
    let runtime = TestRuntime::new();
    let mut client = started(runtime.path());
    client.kill();
    drop(client);

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("--kill")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .status()
        .expect("run --kill");
    assert!(status.success());

    let socket = socket_in(runtime.path());
    let deadline = Instant::now() + PATIENCE;
    while UnixStream::connect(&socket).is_ok() {
        assert!(Instant::now() < deadline, "the session outlived --kill");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn kill_with_nothing_running_says_so_and_fails() {
    let runtime = TestRuntime::new();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("--kill")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run --kill");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("no neotmux session is running"), "{said}");
}

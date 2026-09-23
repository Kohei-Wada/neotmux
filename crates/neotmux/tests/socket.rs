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

/// A test's own `$XDG_RUNTIME_DIR`. `_end_session` is declared first so it drops first —
/// ending whatever session a test left behind, panic or no, before the directory it looked
/// for a lock in is removed.
struct TestRuntime {
    _end_session: session::EndsSession,
    dir: tempfile::TempDir,
}

impl TestRuntime {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("runtime dir");
        let _end_session = session::EndsSession::new(dir.path());
        Self { _end_session, dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

fn socket_in(runtime: &Path) -> PathBuf {
    runtime.join("neotmux/default.sock")
}

/// The runtime layer neotmux ships: the one an install puts on the default search path,
/// read here straight out of the source tree.
fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// A runtime layer of a test's own, written to a fresh temp dir.
fn layer_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("layer dir");
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create layer subdir");
        }
        std::fs::write(&full, content).expect("write layer file");
    }
    dir
}

/// The server's own log, which outlives any one client and so is where a fact about the
/// session — not about one client's screen — is asserted.
fn log_text(runtime: &Path) -> String {
    std::fs::read_to_string(runtime.join("neotmux.log")).unwrap_or_default()
}

fn wait_for_log(runtime: &Path, marker: &str) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let text = log_text(runtime);
        if text.contains(marker) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{marker:?} never reached the log:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
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

    /// A client with the shipped plugins loaded, `plugin/prefix` among them — the only way
    /// for a test to reach `C-b` for real, since [`Client::start`] runs with no runtime
    /// layer at all and so binds no keys.
    fn start_with_prefix(runtime: &Path, shell: &str) -> Self {
        Self::start_with(runtime, shell, 24, 80, &[&shipped_layer()])
    }

    /// The shipped plugins, with a test's own layer stacked in front so it can add a
    /// reporting or reproduction plugin of its own without giving up `C-b`.
    fn start_with_prefix_and_layer(runtime: &Path, shell: &str, layer: &Path) -> Self {
        Self::start_with(runtime, shell, 24, 80, &[layer, &shipped_layer()])
    }

    fn start_sized(runtime: &Path, shell: &str, rows: u16, cols: u16) -> Self {
        Self::start_with(runtime, shell, rows, cols, &[])
    }

    fn start_with(
        runtime: &Path,
        shell: &str,
        rows: u16,
        cols: u16,
        runtime_layers: &[&Path],
    ) -> Self {
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
        if !runtime_layers.is_empty() {
            let joined = runtime_layers
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(":");
            cmd.env("NEOTMUX_RUNTIME", joined);
        }
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

/// Accepted and attached, rather than refused: the whole of what the second client used
/// to be turned away for is now being let in, and what it gets for that is the session.
#[test]
fn a_second_client_is_attached_side_by_side_with_the_first() {
    let runtime = TestRuntime::new();
    let _first = started(runtime.path()); // attached, holding the session while this runs

    // The second connection is greeted, adopts the session, and is shown a frame of it —
    // the first it owes, because no one has looked since the last one it did.
    let mut second = greeted(&socket_in(runtime.path()));
    send_message(
        &mut second,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::ToServer::Attach(an_attach())),
    );
    let got = read_message(&mut second);
    assert!(
        matches!(got, OwnedToClient::Tick(_)),
        "the second client is shown the session, not refused it: {got:?}"
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

/// A float, unlike the text under it on the pane, is the *client's* state: it goes away
/// with the client, and the client that attaches again builds it anew over the same
/// surviving session. The probe announces its float every tick while it is alive, so one
/// new line from the second client — the log outlives client 1 and already holds the old
/// ones — is the proof of the reattach.
#[test]
fn a_reattached_client_builds_the_same_float_again() {
    let runtime = TestRuntime::new();
    // A layer of this test's own, so the probe reaches only this test's two clients and its
    // log lines land only in this test's own log — never in a `shipped_layer()` other
    // sessions' plugins share.
    let layer = layer_with(&[
        ("init.lua", "require('plugin.float_probe')"),
        (
            "lua/plugin/float_probe.lua",
            include_str!("fixtures/lua/plugin/float_probe.lua"),
        ),
    ]);

    // Client 1, with the probe: its float is announced every frame.
    let mut first = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    wait_for_socket(&socket_in(runtime.path()));
    wait_for_log(runtime.path(), "float 4,3 12x4");

    // The seam: the server outlives the client, and client 1's text stays on the shared pane.
    first.send("still-under-the-float\r");
    first.wait_for_output("still-under-the-float");
    first.kill();
    drop(first);

    // Client 2, same probe: the plugin layer re-runs from scratch and the float comes back
    // over the same session. The log is shared, so the point is a *new* announcement.
    let second = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    wait_for_socket(&socket_in(runtime.path()));
    let marker = "float 4,3 12x4";
    let already = log_text(runtime.path()).matches(marker).count();
    let deadline = Instant::now() + PATIENCE;
    loop {
        let seen = log_text(runtime.path());
        if seen.matches(marker).count() > already {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the reattached client never announced the float again:\n{seen}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // And what the session had been holding in the meantime: client 1's text is still on
    // the shared pane, under the new float.
    second.wait_for_output("still-under-the-float");
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

/// Connects and exchanges the greeting, leaving a stream ready to send whatever a real
/// client would send next.
fn greeted(socket: &Path) -> UnixStream {
    let mut stream = UnixStream::connect(socket).expect("connect");
    stream.write_all(&wire::encode_hello()).expect("greet");
    let mut hello = [0u8; HELLO_LEN];
    stream.read_exact(&mut hello).expect("be greeted");
    stream
}

/// Sends `payload` as one framed message.
fn send_message(stream: &mut UnixStream, payload: &[u8]) {
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

/// Greets, then sends the given bytes as one framed message, then hangs up.
fn greet_and_send_message(socket: &Path, payload: &[u8]) {
    let mut stream = greeted(socket);
    send_message(&mut stream, payload);
}

/// A valid `Attach`, small enough that nothing about it is the point of whatever test sends
/// it.
fn an_attach() -> neotmux_proto::msg::Attach {
    neotmux_proto::msg::Attach {
        size: neotmux_proto::Size { rows: 24, cols: 80 },
        first_window: neotmux_proto::WindowId::from(1),
        reserved: neotmux_proto::Reserved::default(),
        gutter: 0,
        wants_pane_output: false,
    }
}

/// One message off a raw link: four length bytes, then the body, then `wire` says what it is.
fn read_message(stream: &mut UnixStream) -> OwnedToClient {
    let mut len = [0u8; 4];
    stream
        .read_exact(&mut len)
        .expect("the server owes a length");
    let n = u32::from_le_bytes(len) as usize;
    let mut body = vec![0u8; n];
    stream
        .read_exact(&mut body)
        .expect("the server owes the body");
    neotmux_proto::wire::decode_to_client(&body).expect("the server's message decodes")
}

/// A raw client into the session: the greet, the attach with the one it is, and the first
/// round it is owed — a round the client is in alone: the only raw one owed its tock, and
/// the real clients in it answer themselves, and so this one takes it whole. The tock it
/// sends carries its own words — the wants and the reserved it attached with.
fn raw_client(attach: neotmux_proto::msg::Attach, runtime: &TestRuntime) -> UnixStream {
    let reserved = attach.reserved;
    let wants = attach.wants_pane_output;
    let mut stream = raw_attach(attach, runtime);
    let _tick = frame_tick(&mut stream);
    frame_tock(&mut stream, wants, reserved);
    frame_done(&mut stream);
    stream
}

/// The greet and the attach only — the first round left for the caller to drive, because
/// that round is shared with the raw clients already in it: a frame waits for every tock
/// it is owed, and the ones in it have to answer in turn, one after the other, before
/// any of them takes its done.
fn raw_attach(attach: neotmux_proto::msg::Attach, runtime: &TestRuntime) -> UnixStream {
    let mut stream = greeted(&socket_in(runtime.path()));
    send_message(
        &mut stream,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::msg::ToServer::Attach(attach)),
    );
    stream
}

/// The half the server owes a raw client in a shared frame: the tick, whatever the frame
/// is for — and none of the done, because the done is owed only after every tock the
/// frame is for has been given. That is why the ones in a shared frame give their tocks
/// first, one after the other, and take their dones last.
fn frame_tick(stream: &mut UnixStream) -> neotmux_proto::msg::Tick {
    match read_message(stream) {
        neotmux_proto::OwnedToClient::Tick(tick) => tick,
        other => panic!("the frame began with {other:?}, not a tick"),
    }
}

/// The half a raw client owes a shared frame: the tock, with this client's words in it —
/// the wants and the reserved are this client's, and the server keeps them per client:
/// the wants OR into the session's, the reserved is the last to say it, in attach order,
/// so a client that asks for either says so in every tock it sends.
fn frame_tock(stream: &mut UnixStream, wants: bool, reserved: neotmux_proto::Reserved) {
    let tock = neotmux_proto::msg::Tock {
        ops: Vec::new(),
        overlay: Vec::new(),
        notices: Vec::new(),
        reserved,
        gutter: 0,
        wants_pane_output: wants,
    };
    send_message(
        stream,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::msg::ToServer::Tock(tock)),
    );
}

/// The done of a shared frame: the frame's last word to this client, owed after every
/// tock it is for has been given, and so it is the last thing a shared round gives. The
/// word is checked, the same way `frame_tick` checks its one: a done that reads as
/// something else is the one a shared round is not over, and the round that is read past
/// it is the one that is not the one the test thought it was in.
fn frame_done(stream: &mut UnixStream) {
    match read_message(stream) {
        neotmux_proto::OwnedToClient::Done(_) => {}
        other => panic!("the frame ended with {other:?}, not a done"),
    }
}

/// One whole frame for a raw client, in the order it is owed: the tick, its tock, the
/// done — a round the client is in alone, the only raw one owed its tock, and the real
/// clients in it answer themselves. `wants` and `reserved` are this client's words for
/// the frame: the wants OR into the session's, the reserved is the last to say it, in
/// attach order.
fn frame_once(
    stream: &mut UnixStream,
    wants: bool,
    reserved: neotmux_proto::Reserved,
) -> neotmux_proto::msg::Tick {
    let tick = frame_tick(stream);
    frame_tock(stream, wants, reserved);
    frame_done(stream);
    tick
}

/// A key to the focused pane, the way a client that is not in a mode gives one: the one
/// its plugins did not consume, and so the session's.
fn type_key(stream: &mut UnixStream, ch: char) {
    send_message(
        stream,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::msg::ToServer::Key(
            neotmux_proto::Key::plain(neotmux_proto::KeyCode::Char(ch)),
        )),
    );
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

/// Finding 4: `attached`'s rule — nothing a client does to its own connection may end
/// somebody's session — was only ever proved for the connection before it attaches. This is
/// the other half: an attached client that sends something `apply_pending` or `await_tock`
/// cannot decode must cost its own connection and nothing else.
#[test]
fn a_malformed_message_from_an_attached_client_costs_its_connection_not_the_session() {
    let runtime = TestRuntime::new();
    let mut first = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    first.send("still-here-after-the-garbage\r");
    first.wait_for_output("still-here-after-the-garbage");
    first.kill();
    drop(first);

    // A raw connection that completes the handshake and attaches for real, so what follows
    // reaches `apply_pending` or `await_tock` rather than `attached`.
    let mut stream = greeted(&socket_in(runtime.path()));
    let attach = wire::encode_to_server(&neotmux_proto::ToServer::Attach(an_attach()));
    send_message(&mut stream, &attach);

    // The attach's own Tick, read (and discarded) so the garbage below is unambiguously the
    // second message of an attached conversation rather than something raced ahead of it.
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let mut tick = [0u8; 64];
    let read = stream
        .read(&mut tick)
        .expect("the tick the attach provokes");
    assert!(read > 0, "no tick followed the attach");

    // A discriminant no message uses: this does not decode at all.
    send_message(&mut stream, &[0xff]);
    drop(stream);

    // Still the same session, for a real client — the bug this pins was the server dying
    // with it, which would leave nothing here to attach to.
    let second = Client::start(runtime.path(), "/bin/cat");
    second.wait_for_output("still-here-after-the-garbage");
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
    // Waited for rather than asserted: a connection the listener never accepted is still
    // queued on it as it closes, and the kernel answers out of that queue until it has torn
    // it down — which under a loaded suite takes longer than the next line. The same wait
    // `sock`'s own test makes, for the same reason.
    let deadline = Instant::now() + PATIENCE;
    while UnixStream::connect(&socket).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the stale socket still had something behind it"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

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

/// Greeted and hung up on without an attach, a connection is dropped, not refused — and
/// neither of those touches the session it found, which is the rule that did not change.
#[test]
fn a_client_that_greets_and_leaves_without_attaching_costs_nothing() {
    let runtime = TestRuntime::new();
    let mut first = started(runtime.path()); // holding the session the whole way

    for _ in 0..3 {
        // A client that is greeted and then closes its end before it could send an attach.
        // It is dropped, quietly: there is no longer a refusal to answer it with.
        let _stream = greeted(&socket_in(runtime.path()));
        // the stream hangs up at the end of this iteration, before it could attach
    }

    // The session survived each of them, and the first client is still talking to it.
    first.send("still-here\r");
    first.wait_for_output("still-here");
}

/// The way out of an immortal session, and the one that does not go through the protocol.
#[test]
fn kill_ends_a_session_nobody_is_attached_to() {
    let runtime = TestRuntime::new();
    let mut client = started(runtime.path());
    client.kill();
    drop(client);

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("kill")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .status()
        .expect("run kill");
    assert!(status.success());

    let socket = socket_in(runtime.path());
    let deadline = Instant::now() + PATIENCE;
    while UnixStream::connect(&socket).is_ok() {
        assert!(Instant::now() < deadline, "the session outlived kill");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `C-b d`: the client exits cleanly, the session does not, and what was in the pane is
/// still there when something attaches to it again.
#[test]
fn a_detach_leaves_the_client_and_keeps_the_session() {
    let runtime = TestRuntime::new();
    let mut first = Client::start_with_prefix(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    first.send("survives-a-detach\r");
    first.wait_for_output("survives-a-detach");
    first.send("\x02d");
    assert!(
        first.wait_for_exit().success(),
        "a detached client did not exit cleanly"
    );

    let second = Client::start_with_prefix(runtime.path(), "/bin/cat");
    second.wait_for_output("survives-a-detach");
}

/// §8: "A window created before a detach and one created after do not share an id." The
/// human path: `C-b c`, `C-b d`, re-attach, `C-b c` again — with a small reporting plugin
/// stacked on the shipped layer so the window ids are on the record rather than the screen.
#[test]
fn a_window_created_before_a_detach_and_one_created_after_do_not_share_an_id() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[
        ("init.lua", "require('plugin.windowreport')"),
        (
            "lua/plugin/windowreport.lua",
            "neotmux.on_tick(function()\n  \
             neotmux.log.info('windows=' .. table.concat(neotmux.window.list(), ','))\n\
             end)\n",
        ),
    ]);

    let mut first = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    wait_for_socket(&socket_in(runtime.path()));
    wait_for_log(runtime.path(), "windows=1");

    first.send("\x02c"); // C-b c: a window made before the detach
    wait_for_log(runtime.path(), "windows=1,2");

    first.send("\x02d"); // C-b d
    assert!(
        first.wait_for_exit().success(),
        "a detached client did not exit cleanly"
    );

    let mut second = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    second.send("\x02c"); // C-b c again: a window made after the re-attach
    let log = wait_for_log(runtime.path(), "windows=1,2,3");
    assert!(
        !log.contains("windows=1,2,1") && !log.contains("windows=1,1,2"),
        "the window made after the re-attach collided with one from before it:\n{log}"
    );
}

/// The mechanism finding 1 fixes, pinned directly: a `resized` event fired on attach is what
/// let a plugin's own `on_event('resized', ...)` handler mint a window id before the
/// watermark caught up. `C-b c` a moment later cannot show this — by the next tick the
/// watermark has already caught up from that tick's own geometry, which is why the test
/// above cannot fail against the old ordering. This one creates the colliding window the
/// same way the bug did: synchronously, inside the event a re-attach used to raise.
#[test]
fn a_resized_handler_on_reattach_does_not_mint_a_window_id_that_already_exists() {
    let runtime = TestRuntime::new();
    // A window made before the detach, so the session has two: a re-attaching client's own
    // `window_ids` starts past 1 regardless of this bug — `attach()` burns it on
    // `first_window`, which a reattach ignores (§4) — so the collision this pins needs a
    // second window already there for the client's un-caught-up watermark to walk into.
    let layer = layer_with(&[(
        "init.lua",
        "neotmux.keymap.set('C-w', function()\n  \
         neotmux.window.focus(neotmux.window.create())\n  return true\nend)\n\
         neotmux.keymap.set('C-t', function()\n  neotmux.detach()\n  return true\nend)\n\
         neotmux.on_event('resized', function()\n  \
         neotmux.window.focus(neotmux.window.create())\nend)\n\
         neotmux.on_tick(function()\n  \
         neotmux.log.info('windows=' .. table.concat(neotmux.window.list(), ','))\nend)\n",
    )]);

    let mut first = Client::start_with(runtime.path(), "/bin/cat", 24, 80, &[layer.path()]);
    wait_for_socket(&socket_in(runtime.path()));
    wait_for_log(runtime.path(), "windows=1");

    first.send("\x17"); // C-w: a second window, made before the detach
    wait_for_log(runtime.path(), "windows=1,2");

    first.send("\x14"); // C-t: detach, bound above rather than through the shipped prefix
    assert!(
        first.wait_for_exit().success(),
        "a detached client did not exit cleanly"
    );

    let second = Client::start_with(runtime.path(), "/bin/cat", 24, 80, &[layer.path()]);
    wait_for_socket(&socket_in(runtime.path()));
    // Long enough for several ticks of the re-attached client's own `resized` handler, if it
    // fires at all, to have run and been logged.
    std::thread::sleep(Duration::from_millis(500));
    drop(second);

    let log = log_text(runtime.path());
    assert!(
        !log.contains("windows=1,2,2"),
        "the re-attach's own `resized` event let a handler mint window 2 a second time:\n{log}"
    );
}

#[test]
fn kill_with_nothing_running_says_so_and_fails() {
    let runtime = TestRuntime::new();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("kill")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run kill");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("neotmux session 'default' is not running"),
        "{said}"
    );
}

/// Two terminals in the session, and a word typed on one that the other has typed
/// nothing: the one the first's plugins did not consume, the one the second sees — the
/// whole of the feature in one sentence. The shell is `cat`, so the word is the pane's
/// own, and the pane is the session's, and the other client is looking at that pane.
/// The marker is one token, because the screen it is looked for in is redrawn with cursor
/// moves between the words of whatever the client has been told.
#[test]
fn a_key_typed_on_one_client_is_seen_on_the_other() {
    let runtime = TestRuntime::new();
    let mut a = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    a.send("the-round-trip-is\r");
    a.wait_for_output("the-round-trip-is");

    let mut b = Client::start(runtime.path(), "/bin/cat");
    b.wait_for_output("the-round-trip-is");

    a.send("typed-on-a\r");
    b.wait_for_output("typed-on-a");
    // In the other direction, because the Review Focus says whichever client: the key
    // one of them types is the session's, and the session is both of theirs.
    b.send("typed-on-b\r");
    a.wait_for_output("typed-on-b");
}

/// One of two goes, and the other is left with the session, because the session is the
/// server's and not either client's. The one that went is gone, and the one that stayed
/// is not told the difference between "the other went" and "nothing happened" — its
/// screen still holds the one that went left behind, and the one it types goes to the
/// same pane.
#[test]
fn a_client_going_costs_the_other_nothing() {
    let runtime = TestRuntime::new();
    let mut a = Client::start(runtime.path(), "/bin/cat");
    let mut b = Client::start(runtime.path(), "/bin/cat");

    a.send("before-a-goes\r");
    b.wait_for_output("before-a-goes");
    a.kill();
    drop(a);

    // The one that is left is looking at the same pane the one that went last typed
    // into, and the one it types is the one that is, the same way the
    // `a_second_client_is_shown_the_session_the_first_one_left` test already proves,
    // for a client that leaves on purpose.
    b.wait_for_output("before-a-goes");
    b.send("still-here\r");
    b.wait_for_output("still-here");

    // And the session itself is still there: a connect that is answered is the server
    // itself saying so.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut answered = false;
    while Instant::now() < deadline && !answered {
        if let Ok(mut stream) = UnixStream::connect(socket_in(runtime.path())) {
            let mut hello = [0u8; neotmux_proto::wire::HELLO_LEN];
            if stream
                .write_all(&neotmux_proto::wire::encode_hello())
                .is_ok()
                && stream.read_exact(&mut hello).is_ok()
            {
                answered = true;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        answered,
        "the session outlives the client that went, and its socket still answers"
    );
}

/// A question one client asks about a pane is answered on that client's link, and on no
/// other: the `Read` and the `Reading` are the two ends of the same question, and the
/// other client had not asked it. The one that is not answered is not refused an error;
/// it is simply not told, which is to say it is never, for the reason it was never
/// asked.
#[test]
fn a_read_is_answered_on_the_link_that_asked_it_only() {
    let runtime = TestRuntime::new();
    let _session = started(runtime.path());

    let mut a = raw_client(
        neotmux_proto::msg::Attach {
            wants_pane_output: false,
            ..an_attach()
        },
        &runtime,
    );
    // B's first round is the one the two of them share, and so it is driven in turn: a
    // frame waits for every tock it is owed, and the one that is not answered is the
    // round that does not finish.
    let mut b = raw_attach(
        neotmux_proto::msg::Attach {
            wants_pane_output: false,
            ..an_attach()
        },
        &runtime,
    );
    let _tick_a = frame_tick(&mut a);
    let _tick_b = frame_tick(&mut b);
    frame_tock(&mut a, false, neotmux_proto::Reserved::default());
    frame_tock(&mut b, false, neotmux_proto::Reserved::default());

    // A's question, on A's link, in this very round: the two of them are done with their
    // tocks when it lands, and so the answer is owed on A's link, and on no other, for
    // the one that is asking is the one it is owed to, in whatever wait the round is in
    // at the time.
    send_message(
        &mut a,
        &neotmux_proto::wire::encode_to_server(&neotmux_proto::msg::ToServer::Read(
            neotmux_proto::msg::Read::Revision {
                pane: neotmux_proto::PaneId::from(1),
            },
        )),
    );
    let got_a = match read_message(&mut a) {
        neotmux_proto::OwnedToClient::Reading(reading) => reading,
        // The round's done landed first — the answer is owed right after it, on the
        // same link, for the two of them are the same round.
        neotmux_proto::OwnedToClient::Done(_) => match read_message(&mut a) {
            neotmux_proto::OwnedToClient::Reading(reading) => reading,
            bad => panic!("A's question is answered right after the round's done: {bad:?}"),
        },
        other => panic!("A's question is answered on A's link, and on no other one: {other:?}"),
    };
    assert!(
        matches!(got_a, neotmux_proto::msg::Reading::Revision(Some(_))),
        "A's question is answered, with the revision it asked for: {got_a:?}"
    );
    frame_done(&mut a);
    frame_done(&mut b);

    // Whatever B is told on its own link from here on may be a tick or a done of the
    // frames the session keeps going — the session is alive, and the frames come — but
    // it must not be a `Reading`, for the question was asked on A's link and on no
    // other.
    b.set_read_timeout(Some(Duration::from_millis(300)))
        .expect("b takes a patience");
    let mut len = [0u8; 4];
    use std::io::Read;
    for _ in 0..8 {
        match b.read_exact(&mut len) {
            Ok(_) => {
                let mut body = vec![0u8; usize::try_from(u32::from_le_bytes(len)).unwrap()];
                b.read_exact(&mut body).expect("b owes a body");
                let message =
                    neotmux_proto::wire::decode_to_client(&body).expect("b's message decodes");
                assert!(
                    !matches!(message, neotmux_proto::OwnedToClient::Reading(_)),
                    "the link that did not ask was answered: {message:?}"
                );
            }
            Err(err)
                if err.kind().eq(&std::io::ErrorKind::WouldBlock)
                    || err.kind().eq(&std::io::ErrorKind::TimedOut) =>
            {
                break;
            }
            Err(_) => break,
        }
    }
}

/// `wants_pane_output` is the OR of the attached clients, and not the last one to say
/// so: the client that asks is the one that keeps the pane delivered while it is in it,
/// and the one that does not ask does not stop the one that does, and the delivery
/// stops with the last one that asked, and not with one before it.
#[test]
fn wants_is_the_or_of_the_clients_and_not_the_last_to_say_so() {
    let runtime = TestRuntime::new();
    let _session = started(runtime.path());

    let mut a = raw_client(
        neotmux_proto::msg::Attach {
            wants_pane_output: true,
            ..an_attach()
        },
        &runtime,
    );
    // The round B comes in on is the one the two of them share, and so it is driven in
    // turn, the tocks first and the dones last: A says the word it attached with, and B
    // says the word it did not.
    let mut b = raw_attach(
        neotmux_proto::msg::Attach {
            wants_pane_output: false,
            ..an_attach()
        },
        &runtime,
    );
    let _tick_a = frame_tick(&mut a);
    frame_tock(&mut a, true, neotmux_proto::Reserved::default());
    let _tick_b = frame_tick(&mut b);
    frame_tock(&mut b, false, neotmux_proto::Reserved::default());
    frame_done(&mut a);
    frame_done(&mut b);

    // A says a word, and the one it says it on is A: the echo of it lands in some
    // round's tick, and the rounds are the ones the two of them share, and so they are
    // driven in turn, round after round, until the one it lands in arrives: A's `wants`
    // is in the OR while A is in it, and B's `false` did not win, which is the one a
    // last-`Tock`-wins shape can say and the OR cannot.
    type_key(&mut a, 'w');
    let deadline = Instant::now() + PATIENCE;
    let mut tick_a = frame_tick(&mut a);
    loop {
        frame_tock(&mut a, true, neotmux_proto::Reserved::default());
        let _tick_b = frame_tick(&mut b);
        frame_tock(&mut b, false, neotmux_proto::Reserved::default());
        frame_done(&mut a);
        frame_done(&mut b);
        if tick_a.steps.iter().any(|step| {
            matches!(
                step,
                neotmux_proto::msg::Step::Event(neotmux_proto::Event::PaneOutput { .. })
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the word A typed never reached a tick while A was asking: {tick_a:?}"
        );
        tick_a = frame_tick(&mut a);
    }

    // A goes, and the last one asking goes with it: the OR is the empty set now, and A's
    // `true` is not the last word in it, and B's `false` is not the first.
    drop(a);
    type_key(&mut b, 'w');
    let tick_b_after = frame_once(&mut b, false, neotmux_proto::Reserved::default());
    assert!(
        !tick_b_after.steps.iter().any(|step| matches!(
            step,
            neotmux_proto::msg::Step::Event(neotmux_proto::Event::PaneOutput { .. })
        )),
        "the last client that asked went with A, and the delivery went with it: {tick_b_after:?}"
    );
}

/// The last client to attach is the one the session is told to be: the bands B reserves
/// are the bands the session keeps, the surface it is laid out against, and A is looking
/// at the same one, which is to say the geometry A has been told is the one B's tock
/// said, and not the one it did.
#[test]
fn the_last_client_to_attach_is_the_one_the_session_is_told() {
    let runtime = TestRuntime::new();
    let _session = started(runtime.path());

    let mut a = raw_client(
        neotmux_proto::msg::Attach {
            wants_pane_output: false,
            ..an_attach()
        },
        &runtime,
    );
    let band = neotmux_proto::Reserved::horizontal(0, 1); // one row, from the bottom
    let mut b = raw_attach(
        neotmux_proto::msg::Attach {
            reserved: band,
            ..an_attach()
        },
        &runtime,
    );

    // The round B comes in on is the one the two of them share, and so it is driven in
    // turn, the tocks first and the dones last: A says the word it attached with, and B
    // says the word it did not. B is after A in it, and so the band is the word the
    // session keeps: the reserved is the last to say it, in attach order — and it is
    // said at the attach, before the round begins, and so it is in the round's ticks,
    // the ones the session tells to everybody, and not in the tocks, because B does
    // not change it, and only the changed is told.
    let tick_a = frame_tick(&mut a);
    let tick_b = frame_tick(&mut b);
    frame_tock(&mut a, false, neotmux_proto::Reserved::default());
    frame_tock(&mut b, false, band);
    frame_done(&mut a);
    frame_done(&mut b);

    // And the ticks are saying it: the round B came in on, and the one A was in, for
    // the two of them were in the same round, and so they were told it together — the
    // bands B's attach said are the bands in it, and the surface is the one that band
    // takes the session's rows to, 24 the one the client attached with, 23 the one with
    // the band in it, and not the one it was without the band.
    assert!(
        tick_b.steps.iter().any(|step| {
            matches!(
                step,
                neotmux_proto::msg::Step::Bands { applied, .. } if *applied == band
            )
        }),
        "the bands B's attach said are the bands the session keeps: {tick_b:?}"
    );
    assert_eq!(
        tick_a.geometry.size.rows, 23,
        "the surface is the one with the band in it, the 24 rows the client attached
         with minus the one B reserved: {tick_a:?}"
    );
}

/// One connection's silence is its own: the door reads a connection only as far as it has
/// spoken, so a peer that greets and then says nothing holds up nobody's frame.
///
/// The frame is what puts a shell's output on a screen, so the client that is attached and
/// typing is the one that says whether the door cost anything: a door that ran the handshake
/// inline would hold this client's next frame for as long as the silence lasted — which, for
/// a peer that never sends its attach, is for ever.
#[test]
fn a_connection_that_says_nothing_costs_the_attached_client_nothing() {
    let runtime = TestRuntime::new();
    let mut client = started(runtime.path());

    // Two kinds of silence, because the door has two reads in it: one that never says
    // hello, and one that says hello and never attaches.
    let _mute = UnixStream::connect(socket_in(runtime.path())).expect("connect");
    let _silent = greeted(&socket_in(runtime.path()));

    client.send("the-silent-ones-cost-nothing\r");
    client.wait_for_output("the-silent-ones-cost-nothing");
}

/// A length prefix nobody could mean is one client's own doing, and it costs that client's
/// connection and nothing else.
///
/// The link cannot be read past such a prefix, so the connection is over either way; what is
/// asserted here is the other half — that the session, the other client and the shells behind
/// it go on. This is the same deal the undecodable message already got, at the boundary one
/// step before it: with a second client attached, letting it out would be one client ending
/// another's shells with a four-byte write.
#[test]
fn one_client_s_impossible_message_costs_that_client_and_no_other() {
    let runtime = TestRuntime::new();
    let mut client = started(runtime.path());

    // Attached, in a frame the session is waiting on, and then a prefix the link cannot be
    // read past. Held to the end of the test: a connection that closed would be an ordinary
    // hangup, which was never the thing in question.
    let mut bad = raw_client(an_attach(), &runtime);
    bad.write_all(&u32::MAX.to_le_bytes())
        .expect("the prefix nobody could mean");
    bad.flush().expect("flush");

    client.send("the-impossible-message-costs-one\r");
    client.wait_for_output("the-impossible-message-costs-one");

    // And the session is the one that outlived it: the client above is being served by it.
    let _ = &bad;
}

# One Server, Many Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One socket for a user, one server behind it, and as many sessions in it as you have
started. `ls` and `kill` become questions asked of the server rather than a directory walk and
a signal.

**Architecture:** Sessions gain names, and `Attach` carries the one the client asked for: the
server hands back the session with that name or makes it. The socket stops being per session
and becomes the server's. A connection that sends `List` or `Kill` instead of `Attach` is not a
client — the doorstep already tells them apart — so it is answered and closed. `neotmux.session_name()`
answers the session whose dispatch is running, which is the first piece of the API the sidebar
will use.

**Tech Stack:** Rust 2024, one workspace of five crates, `mlua` (Lua 5.4).

**Spec:** `docs/superpowers/specs/2026-09-23-sessions-in-the-server-design.md` — stage 2 of its
§11 as reordered, covering §4 (the socket), §5 (the CLI) and the N-session half of §9/§10.

## Global Constraints

- The two-name behaviour `crates/neotmux/tests/sessions.rs` pins — `ls` lists both, `kill work`
  leaves `default` alone, a second client of the same name gets the first's pane — **holds at
  every commit**. What changes is that those are two sessions in one server rather than two
  servers. Assertions do not move; the two helpers that observe them through
  `<name>.sock` do.
- Exactly one Lua layer, and it keeps running with no client attached.
- `neotmux-lua` keeps `neotmux-proto` as its only workspace dependency.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` green at every commit, read rather than assumed.
- English for code, comments and commits; comments only where the WHY is non-obvious.
- Commit messages in this repo's voice, ending with the two attribution lines recent commits
  carry.

## The shape of the staging

Tasks 1-3 are invisible: a session gains a name it always had implicitly, the API answers it,
and the wire learns two questions nobody asks yet. Task 4 is the switch — one socket, the CLI
over the protocol, more than one session in the map — and it is one commit because a socket
consolidated over a map that still holds one session is the broken window this plan's
predecessor was reordered to avoid.

---

## File Structure

| File | Responsibility afterwards |
| --- | --- |
| `crates/neotmux-proto/src/msg.rs` | `Attach` names a session; `List`/`Kill` up, `Sessions`/`Killed` down |
| `crates/neotmux-proto/src/wire.rs` | Their encoding |
| `crates/neotmux-lua/src/ambient.rs` | Each session's name, beside its geometry |
| `crates/neotmux-lua/src/api/mod.rs` | `session_name()` reads the bound session |
| `crates/neotmux/src/sock.rs` | One socket and one lock, for the server |
| `crates/neotmux/src/server.rs` | Sessions by name; the door answers the two questions |
| `crates/neotmux/src/main.rs` | `ls` and `kill` ask the server |
| `crates/neotmux/tests/sessions.rs` | The same assertions, observed through `ls` |

---

### Task 1: A session has a name

**Files:**
- Modify: `crates/neotmux-proto/src/msg.rs`, `crates/neotmux-proto/src/wire.rs`,
  `crates/neotmux-proto/src/lib.rs` (`PROTOCOL_VERSION`)
- Modify: `crates/neotmux/src/server.rs`
- Modify: `crates/neotmux/src/tui.rs`, `crates/neotmux/src/main.rs` (the client says which)
- Modify: `crates/neotmux/tests/socket.rs` (`an_attach`)

**Interfaces:**
- Produces: `Attach { size: Size, session: String }`; `server::Sessions` holding named
  sessions, with `id_of(&str)`, `names()`, `get_mut(SessionId)`.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux/tests/socket.rs`, beside the other raw-client tests:

```rust
/// A client says which session it wants, and a second client saying the same name is given
/// the same one rather than a fresh one. The name is the client's only say in the matter:
/// everything else about a session is the server's.
#[test]
fn two_clients_naming_the_same_session_are_given_the_same_one() {
    let runtime = TestRuntime::new();
    let mut session = started(runtime.path());
    session.send("shared-by-name\r");
    session.wait_for_output("shared-by-name");

    let mut raw = raw_attach(
        neotmux_proto::msg::Attach {
            size: neotmux_proto::Size { rows: 24, cols: 80 },
            session: "default".to_string(),
        },
        &runtime,
    );
    let done = next_done(&mut raw);
    let frame = done.frame.expect("the session it named");
    let text: String = frame
        .panes
        .iter()
        .flat_map(|pane| pane.grid.cells.iter())
        .map(|cell| cell.text.as_str())
        .collect();
    assert!(
        text.contains("shared-by-name"),
        "the named client was given a fresh session rather than the one it named"
    );
}
```

`OwnedDone::frame` is an `OwnedFrame`, whose `panes` are `OwnedPaneFrame`s holding a
`GridSnapshot`; a snapshot's `cells` are `SnapCell`s and a cell's glyph is its `text`. Joining
every cell of every pane gives the screen with no line breaks in it, which is all this
assertion needs.

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux --test socket two_clients_naming_the_same_session`
Expected: FAIL to compile — `Attach` has no `session`.

- [ ] **Step 3: The wire**

```rust
pub struct Attach {
    pub size: Size,
    /// Which session this client is asking for. The server hands back the one with that name
    /// or makes it, which is what `neotmux <name>` has always meant.
    pub session: String,
}
```

`encode_to_server` writes `w.str(&attach.session)` after the size; `decode_to_server` reads
`r.string()?`. Bump `PROTOCOL_VERSION` to 5.

- [ ] **Step 4: The client says which**

`tui::open` takes the name and puts it in the `Attach`. `client()` in `main.rs` already has
`session: Option<String>` and resolves the default with
`neotmux::sock::DEFAULT_SESSION` — pass that resolved name down rather than resolving it twice.

- [ ] **Step 5: Sessions are named**

In `server.rs`:

```rust
/// One session and the name it answers to.
struct Held {
    name: String,
    session: Session,
}

#[derive(Default)]
struct Sessions {
    live: BTreeMap<SessionId, Held>,
    next: u32,
}

impl Sessions {
    fn mint(&mut self) -> SessionId {
        self.next += 1;
        SessionId::from(self.next)
    }

    fn id_of(&self, name: &str) -> Option<SessionId> {
        self.live
            .iter()
            .find(|(_, held)| held.name == name)
            .map(|(id, _)| *id)
    }

    /// Every session's name, in name order, which is the order `ls` prints them in.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.live.values().map(|held| held.name.clone()).collect();
        names.sort();
        names
    }

    fn get_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.live.get_mut(&id).map(|held| &mut held.session)
    }
}
```

`admit` stops taking whichever session is there and takes the one named:

```rust
    let session = match sessions.id_of(&attach.session) {
        Some(id) => {
            if let Some(session) = sessions.get_mut(id) {
                session.adopt(&setup)?;
            }
            id
        }
        None => {
            let id = sessions.mint();
            sessions.live.insert(
                id,
                Held {
                    name: attach.session.clone(),
                    session: Session::new(&setup, waker.clone(), terminals.clone())?,
                },
            );
            id
        }
    };
```

A name the server would not accept is refused at the door rather than made into a session:
`neotmux::sock::validate_session_name` is the same law the CLI enforces, and a peer that did
not go through the CLI is not exempt. Refuse with the message it gives, the way the door
already refuses a first message that is not an attach.

Every other `sessions.live.get_mut(&id)` in the file becomes `sessions.get_mut(id)`.

- [ ] **Step 6: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS, the new test included. Nothing observable has changed: one socket per session
still means the name a client sends is the name of the only session behind that socket.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "a client says which session it wants"
```

---

### Task 2: `session_name()` answers the session in hand

**Files:**
- Modify: `crates/neotmux-lua/src/ambient.rs`, `crates/neotmux-lua/src/lib.rs`,
  `crates/neotmux-lua/src/api/mod.rs`
- Modify: `crates/neotmux/src/server.rs` (naming a session as it is made)
- Modify: `crates/neotmux-lua/tests/session_name.rs`,
  `crates/neotmux/tests/fixtures/lua/plugin/session_name.lua`
- Modify: `docs/plugins.md`

**Interfaces:**
- Consumes: `Sessions` (Task 1).
- Produces: `Ambient::name_of(SessionId) -> Option<String>`, `Ambient::name(SessionId, String)`;
  `LuaSubsystem::name_session(&self, session: SessionId, name: &str)`;
  `LuaSubsystem::build(layers, panes)` and `build_with(layers, panes, clipboard)` — the
  `build_named` that took one name for the whole layer goes.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/session_name.rs` becomes two tests with the same subjects and a
mechanism that exists:

```rust
//! `neotmux.session_name()`: which session the work in hand is for. One Lua layer serves all
//! of them, so the answer is a property of the dispatch rather than of the layer.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;
use neotmux_proto::SessionId;

mod support;

fn named(init: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn logged(sub: &LuaSubsystem, session: Option<SessionId>) -> Vec<String> {
    sub.drain_ops(session)
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::ops::Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A tick is told the name of the session it is for, and two sessions are told two names by
/// the one layer.
#[test]
fn a_tick_is_told_the_name_of_the_session_it_is_for() {
    let (_dir, sub) = named(
        "neotmux.on_tick(function() neotmux.log.info('name=' .. neotmux.session_name()) end)",
    );
    let work = support::one();
    let home = SessionId::from(2);
    sub.name_session(work, "work");
    sub.name_session(home, "home");

    sub.dispatch_tick(work);
    sub.dispatch_tick(home);

    assert_eq!(logged(&sub, Some(work)), vec!["name=work".to_string()]);
    assert_eq!(logged(&sub, Some(home)), vec!["name=home".to_string()]);
}

/// A load runs before any session exists, so there is no name to answer with. `nil` is the
/// honest answer and the one every other per-session fact already gives at load: the geometry
/// is empty there for the same reason.
#[test]
fn a_load_has_no_session_to_name() {
    let (_dir, sub) = named(
        "neotmux.log.info('at load ' .. tostring(neotmux.session_name()))",
    );
    assert_eq!(logged(&sub, None), vec!["at load nil".to_string()]);
}
```

- [ ] **Step 2: Run them**

Run: `cargo test -p neotmux-lua --test session_name`
Expected: FAIL — `name_session` does not exist, and `build_named` is what the file used to call.

- [ ] **Step 3: Names live beside the geometry**

`Ambient` gains `names: RefCell<BTreeMap<SessionId, String>>`, with

```rust
    /// What a session is called. Set once, when the host makes it.
    pub fn name(&self, session: SessionId, name: &str) {
        self.names
            .borrow_mut()
            .insert(session, name.to_string());
    }

    /// The bound session's name, or `None` outside any dispatch.
    pub fn current_name(&self) -> Option<String> {
        let current = self.current.get()?;
        self.names.borrow().get(&current).cloned()
    }
```

and `forget` drops the name with everything else.

- [ ] **Step 4: The API reads it**

`api::Shared`'s `session_name: String` becomes `ambient: Rc<crate::ambient::Ambient>`, and the
installed function becomes:

```rust
    {
        let ambient = shared.ambient.clone();
        let session_name = lua.create_function(move |_lua, (): ()| {
            Ok::<Option<String>, mlua::Error>(ambient.current_name())
        })?;
        neotmux.set("session_name", session_name)?;
    }
```

`LuaSubsystem::build_named` loses its name parameter and becomes `build_with(layers, panes,
clipboard)`; `build(layers, panes)` keeps calling it with `clipboard::Absent`. `name_session`
is one line onto the ambient. Every caller — `server.rs`, and the tests that pass a clipboard
— follows.

- [ ] **Step 5: The server names what it makes**

In `admit`'s `None` arm, right after the session is inserted:

```rust
            plugins.lua().name_session(id, &attach.session);
```

- [ ] **Step 6: The E2E fixture asks from a tick**

`crates/neotmux/tests/fixtures/lua/plugin/session_name.lua` calls `session_name()` at file
scope, which is load time. Move the call into a tick, which is where the fact now exists:

```lua
-- The name of the session this tick is for, in the log: the E2E proof that the fact reaches
-- the plugins at all. Asked from a tick rather than at load, because a load happens before
-- any session does.
local said = false
neotmux.on_tick(function()
  if not said then
    said = true
    neotmux.log.info('session_name ' .. neotmux.session_name())
  end
end)
```

`fixtures.rs`'s `a_named_client_is_told_its_name_by_the_api` waits for `session_name default`
in the log and is untouched.

- [ ] **Step 7: The docs**

`docs/plugins.md:658` says "the name this client attached under; 'default' where it had none".
It is the session the work in hand is for, and `nil` during a load. Say that.

- [ ] **Step 8: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "the name of the session the work in hand is for"
```

---

### Task 3: The two questions that are not an attach

Nothing sends these yet. Task 4 does, from the CLI.

**Files:**
- Modify: `crates/neotmux-proto/src/msg.rs`, `crates/neotmux-proto/src/wire.rs`
- Modify: `crates/neotmux/src/server.rs`

**Interfaces:**
- Produces: `ToServer::List`, `ToServer::Kill { session: String }`;
  `ToClient::Sessions(&'a [String])`, `ToClient::Killed(bool)`, and their owned halves.

- [ ] **Step 1: Round-trip them**

In `wire.rs`'s tests, beside the clipboard round trip:

```rust
    #[test]
    fn the_two_questions_that_are_not_an_attach_round_trip() {
        round_trip_to_server(ToServer::List);
        round_trip_to_server(ToServer::Kill {
            session: "work".to_string(),
        });

        let names = vec!["default".to_string(), "work".to_string()];
        let bytes = encode_to_client(&ToClient::Sessions(&names));
        assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Sessions(names)));

        for found in [true, false] {
            let bytes = encode_to_client(&ToClient::Killed(found));
            assert_eq!(decode_to_client(&bytes), Ok(OwnedToClient::Killed(found)));
        }
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux-proto the_two_questions`
Expected: FAIL — the variants do not exist.

- [ ] **Step 3: Implement**

New tags `TO_SERVER_LIST = 5`, `TO_SERVER_KILL = 6`, `TO_CLIENT_SESSIONS = 6`,
`TO_CLIENT_KILLED = 7`, following the `Bye` encoding for the string list. `PROTOCOL_VERSION`
is already moving in Task 1; it does not move twice for one release of work, so leave it.

- [ ] **Step 4: The door answers them**

`Doorstep` gains a variant:

```rust
    /// It is not a client at all: it asked a question about the sessions, and the answer is
    /// the whole of what it wanted. §9's command channel is this shape widened.
    Asking(Ask),
```

with

```rust
enum Ask {
    List,
    Kill { session: String },
}
```

`doorstep`'s decode arm maps `ToServer::List` and `ToServer::Kill` to it, and `door` answers
and drops the connection:

```rust
            Doorstep::Asking(ask) => answer(&mut one.link, ask, plugins, sessions, clients),
```

```rust
/// A connection that asked rather than attached, answered and let go. It is not a client, so
/// it never enters `clients` and never sees a frame.
fn answer(
    link: &mut Link,
    ask: Ask,
    plugins: &Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
) {
    match ask {
        Ask::List => {
            let names = sessions.names();
            let _ = link.send(&wire::encode_to_client(&ToClient::Sessions(&names)));
        }
        Ask::Kill { session } => {
            let found = match sessions.id_of(&session) {
                Some(id) => {
                    end_session(id, plugins, sessions, clients);
                    true
                }
                None => false,
            };
            let _ = link.send(&wire::encode_to_client(&ToClient::Killed(found)));
        }
    }
}

/// A session ends: its clients are told, its shells go with it — `Shells::drop` closes every
/// child — and the Lua layer forgets what it was holding for it.
fn end_session(
    id: SessionId,
    plugins: &Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
) {
    for client in clients.iter_mut().filter(|client| client.session == id) {
        let lines = plugins.notifications().since(client.mark);
        let _ = client
            .link
            .send(&wire::encode_to_client(&ToClient::Bye(&lines)));
        let _ = client.link.send(&wire::encode_to_client(&ToClient::Done(&Done {
            steps: Vec::new(),
            frame: None,
            end: Some(End::Quit),
        })));
        client.gone = true;
    }
    clients.retain(|client| !client.gone);
    sessions.live.remove(&id);
    plugins.lua().forget_session(id);
}
```

`frame`'s own `Some(_) => { ... }` arm becomes a call to `end_session` followed by
`over = sessions.live.is_empty()`: a session that quits or runs out of shells ends the server
only when it was the last one. That is the behaviour change N sessions make, and it is the one
`kill_work_leaves_default_running_and_vica_versa` is about.

- [ ] **Step 5: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS. With one session per server, "the last one" is always the only one, so nothing
observable moves yet.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "a connection that asks about sessions rather than attaching to one"
```

---

### Task 4: One socket

The switch. The socket stops being per session, `ls` and `kill` ask the server, and two names
become two sessions in one server.

**Files:**
- Modify: `crates/neotmux/src/sock.rs`
- Modify: `crates/neotmux/src/main.rs`, `crates/neotmux/src/start.rs`
- Modify: `crates/neotmux/tests/session/mod.rs`, `crates/neotmux/tests/sessions.rs`,
  `crates/neotmux/tests/socket.rs`

- [ ] **Step 1: One path**

`Paths::resolve(env, name)` loses the name:

```rust
    /// The socket and lock a client and its server agree on. One pair for a user: the
    /// sessions are inside the server, and finding them is a question it answers.
    pub fn resolve(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> anyhow::Result<Self> {
        let dir = runtime_dir(env);
        make_dir(&dir).with_context(|| format!("prepare {}", dir.display()))?;
        Ok(Self {
            socket: dir.join("server.sock"),
            lock: dir.join("server.lock"),
        })
    }
```

The module doc's second paragraph argues for a file per session — "killing one, finding one, or
listing them is `ls` rather than a protocol". Replace it with why that stopped being true: a
sidebar that shows what each session is doing has to read a session it is not attached to, and
one process is the one that does not need a second protocol to do it.

`socket_dir` goes: `list_sessions` was its only caller and is about to stop walking a
directory. `resolve_builds_paths_for_the_name_asked_for` becomes
`resolve_builds_the_one_pair_a_user_has`, asserting `neotmux/server.sock` and
`neotmux/server.lock`.

- [ ] **Step 2: `ls` and `kill` ask**

In `main.rs`, one helper both use:

```rust
/// Asks the server one question and reads its one answer. `None` is nothing listening, which
/// is not an error here: it is the answer to "what is running".
fn ask(msg: &neotmux_proto::msg::ToServer) -> anyhow::Result<Option<OwnedToClient>> {
    let paths = neotmux::sock::Paths::resolve(&|key| env::var_os(key))?;
    let Some(stream) = neotmux::sock::connect(&paths.socket)? else {
        return Ok(None);
    };
    let mut link = neotmux::link::Link::open(stream).context("greet the neotmux server")?;
    link.send(&neotmux_proto::wire::encode_to_server(msg))
        .context("ask the neotmux server")?;
    let Some(message) = link.recv().context("read the server's answer")? else {
        anyhow::bail!("the neotmux server went away without answering");
    };
    Ok(Some(
        neotmux_proto::wire::decode_to_client(&message).context("decode the server's answer")?,
    ))
}
```

`list_sessions` sends `ToServer::List`. `None`, or `Sessions` with nothing in it, is the
existing `anyhow::bail!("no neotmux sessions are running")`. Otherwise one line per name, with
the pid the lock file gives — the same line shape as today, and the pid is the server's, which
is the process a person would signal.

`kill_session(name)` sends `ToServer::Kill { session: name }`. `Killed(true)` prints
`neotmux: asked the session '<name>' (pid <pid>) to end`; `Killed(false)` and `None` both
`bail!("neotmux session '<name>' is not running")`. The `Holder`/`holder` path goes from
`kill_session` but stays in `sock.rs`: `tests/session/mod.rs` uses it to end the server a test
started, which is still exactly what it is for.

- [ ] **Step 3: `--server` stops taking a name**

The server has no name of its own any more. `parse_args`'s `--server` arm drops its
name-swallowing branch and `start::spawn` stops passing one; `start::serve` takes no name and
`server::serve` loses the `name` parameter it took for `build_named`, which Task 2 removed.

- [ ] **Step 4: The tests observe through `ls`**

`crates/neotmux/tests/sessions.rs`'s `up` and `wait_for_gone` poll `<name>.sock`. There is no
such file. They ask instead:

```rust
/// Whether the server says this session is running. The socket is the server's now, so a
/// session's existence is a question rather than a file.
fn running(rt: &Runtime, name: &str) -> bool {
    let (code, out, _) = ls(rt);
    code == 0 && out.lines().any(|line| line.starts_with(name))
}

fn up(rt: &Runtime, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !running(rt, name) {
        assert!(
            Instant::now() < deadline,
            "the session '{name}' never came up"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_gone(rt: &Runtime, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while running(rt, name) {
        assert!(
            Instant::now() < deadline,
            "the session '{name}' outlived kill"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
```

Every assertion in that file stays exactly as it is.

`crates/neotmux/tests/socket.rs`'s `socket_in` returns `neotmux/server.sock`, and `an_attach`
names `"default"`.

`crates/neotmux/tests/session/mod.rs` calls `Paths::resolve(env, DEFAULT_SESSION)`; it drops
the name. Its comment says it ends "the session"; it ends the server, which is now a different
sentence.

- [ ] **Step 5: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS, `sessions.rs` included — with two sessions in one server rather than two
servers, and not one assertion in that file changed.

Watch `kill_work_leaves_default_running_and_vica_versa` in particular: it is the test that
could only be answered by two processes before and is now answered by one map.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "one socket, and the sessions behind it"
```

---

### Task 5: What is true now

**Files:**
- Modify: `docs/superpowers/specs/2026-09-23-sessions-in-the-server-design.md`
- Modify: `docs/plugins.md` if anything in it still says a session is a process

- [ ] **Step 1: Mark the stage done**

§11's reordered list gets stage 2 marked done, naming this plan. §4 and §5 described the
socket and the CLI as proposals; say what landed, including the two questions the wire
carries (`List`, `Kill`) which §6's protocol table does not list.

- [ ] **Step 2: The herdr table**

§1's table has four rows and two of them said yes. The fourth — "all the sessions in one view,
and a way into any of them" — is half done: they are all in one server and `ls` lists them, but
the view and the way in are Lua's, and Lua cannot ask yet. Say exactly that rather than ticking
the row.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "the spec says what landed"
```

---

## What this plan does not do

- `neotmux.session.list` / `current` / `with` / `switch` / `create` / `kill`, and the sidebar
  that uses them. That is the next plan, and it is what makes §1's fourth row true.
- The command surface (`neotmux.command`, the CLI reaching it, the agent-facing API). Its own
  spec.
- Per-session options, several servers, `kill-server`. Still non-goals.

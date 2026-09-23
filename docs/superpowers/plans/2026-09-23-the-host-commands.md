# The host commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `neotmux new`, `rename`, `switch`, `windows`, `panes`, `send` and `capture` — the
commands that control the server, answered in Rust without the Lua layer being consulted.

**Architecture:** A third message shape on the socket (`ToServer::Command`), answered at the
door beside `List` and `Kill`, and a `ToClient::Ran` carrying lines and an ok flag. The CLI
gains one generic path: parse a command name and its arguments, ask, print the lines, exit 0
or 1.

**Tech Stack:** Rust 2024, one workspace, no new dependencies. `portable-pty` for the shells
that already exist; `tempfile` and `CARGO_BIN_EXE_neotmux` in the socket tests.

**Spec:** `docs/superpowers/specs/2026-09-23-the-command-surface-design.md`

## Global Constraints

- `PROTOCOL_VERSION` goes to **9** in `crates/neotmux-proto/src/lib.rs`, once, in Task 1.
- A host command **never consults, enters or wakes the Lua layer**. It is answered in
  `server::answer`, before any frame.
- Output is **lines on stdout**; failure is a **non-zero exit and a sentence on stderr**. No
  JSON, no flags for it.
- `-t <name>` names the session and defaults to `neotmux::sock::DEFAULT_SESSION`, the same
  rule an attach follows.
- `send` and `capture` take a **pane id and no `-t`**: pane ids come from one server-wide
  `PaneIds` counter, so `%4` names one pane in one session.
- Naming a thing that is not there — a session, a pane, a command — is a **failure**, not an
  empty answer.
- Scope is the spec's stages 1–3. `neotmux.command` and Lua dispatch (stage 4) are a separate
  plan and nothing here may anticipate them beyond what §5 of the spec already fixes.
- Every task ends green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test --workspace`.
- Commit messages are prose in this repo's voice: a lower-case summary line that says what is
  true now, then why. No `feat:` prefixes.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/neotmux-proto/src/msg.rs` | `ToServer::Command`, `ToClient::Ran`, `OwnedToClient::Ran` |
| `crates/neotmux-proto/src/wire.rs` | their codec and discriminants |
| `crates/neotmux-proto/src/lib.rs` | `PROTOCOL_VERSION` |
| `crates/neotmux/src/command.rs` | **new** — the host command table: parsing a name and args into a `Command`, and running one against the sessions |
| `crates/neotmux/src/server.rs` | the door arm, `Ask::Command`, `answer` |
| `crates/neotmux/src/session.rs` | the accessors a command reads: `windows_list`, `pane_rows`, `write_to_pane` |
| `crates/neotmux/src/main.rs` | argument parsing, `run_command`, the usage text |
| `crates/neotmux/tests/socket.rs` | the commands against a live server |

`command.rs` is new rather than more of `server.rs` because it is the one part of this with no
socket, no door and no frame in it: a name, some arguments, the sessions, and lines back. That
makes it the part a test can drive directly.

---

### Task 1: The wire carries a command

**Files:**
- Modify: `crates/neotmux-proto/src/msg.rs`
- Modify: `crates/neotmux-proto/src/wire.rs`
- Modify: `crates/neotmux-proto/src/lib.rs`

**Interfaces:**
- Produces: `ToServer::Command { session: Option<String>, name: String, args: Vec<String> }`,
  `ToClient::Ran { ok: bool, lines: &[String] }`, `OwnedToClient::Ran { ok: bool, lines:
  Vec<String> }`.

- [ ] **Step 1: Write the failing round-trip test**

In `crates/neotmux-proto/src/wire.rs`, in the existing `mod tests`, beside
`the_two_questions_that_are_not_an_attach_round_trip`:

```rust
#[test]
fn a_command_and_its_answer_round_trip() {
    let asked = ToServer::Command {
        session: Some("work".to_string()),
        name: "send".to_string(),
        args: vec!["%4".to_string(), "cargo test".to_string()],
    };
    assert_eq!(decode_to_server(&encode_to_server(&asked)), Ok(asked));

    let lines = vec!["%1 @1 80x24 bash /home/kohei".to_string()];
    let answered = encode_to_client(&ToClient::Ran { ok: true, lines: &lines });
    assert_eq!(
        decode_to_client(&answered),
        Ok(OwnedToClient::Ran { ok: true, lines })
    );
}

/// A command with no session named is the default session's, and `None` has to survive the
/// wire as `None` rather than as an empty name: `""` is a name the door would refuse.
#[test]
fn a_command_with_no_session_named_keeps_its_none() {
    let asked = ToServer::Command {
        session: None,
        name: "ls".to_string(),
        args: Vec::new(),
    };
    assert_eq!(decode_to_server(&encode_to_server(&asked)), Ok(asked));
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux-proto a_command_ 2>&1 | tail -20`
Expected: FAIL — `no variant named Command`.

- [ ] **Step 3: Add the variants**

In `msg.rs`, in `ToServer`, after `Kill`:

```rust
    /// Not a client either: a connection asking the server to run a named command. The
    /// answer is the whole of what it wanted, and it is let go afterwards.
    Command {
        /// The session the command is addressed to. `None` is the default name, the same
        /// rule an attach follows.
        session: Option<String>,
        name: String,
        args: Vec<String>,
    },
```

In `ToClient`, after `Killed`:

```rust
    /// What a command did. `lines` is stdout where `ok`, and the one sentence that says why
    /// not where it is not. Never both.
    Ran { ok: bool, lines: &'a [String] },
```

In `OwnedToClient`, after its `Killed(bool)`:

```rust
    Ran { ok: bool, lines: Vec<String> },
```

- [ ] **Step 4: Add the codec**

In `wire.rs`, beside the other discriminants:

```rust
const TO_SERVER_COMMAND: u8 = 7;
const TO_CLIENT_RAN: u8 = 8;
```

In `encode_to_server`:

```rust
        ToServer::Command {
            session,
            name,
            args,
        } => {
            w.u8(TO_SERVER_COMMAND);
            w.bool(session.is_some());
            w.str(session.as_deref().unwrap_or(""));
            w.str(name);
            w.len(args.len());
            for arg in args {
                w.str(arg);
            }
        }
```

In `decode_to_server`:

```rust
        TO_SERVER_COMMAND => {
            let named = r.bool()?;
            let session = r.string()?;
            let name = r.string()?;
            let mut args = Vec::new();
            for _ in 0..r.len()? {
                args.push(r.string()?);
            }
            ToServer::Command {
                session: named.then_some(session),
                name,
                args,
            }
        }
```

In `encode_to_client`:

```rust
        ToClient::Ran { ok, lines } => {
            w.u8(TO_CLIENT_RAN);
            w.bool(*ok);
            w.len(lines.len());
            for line in *lines {
                w.str(line);
            }
        }
```

In `decode_to_client`, beside `TO_CLIENT_SESSIONS`:

```rust
        TO_CLIENT_RAN => {
            let ok = r.bool()?;
            let mut lines = Vec::new();
            for _ in 0..r.len()? {
                lines.push(r.string()?);
            }
            OwnedToClient::Ran { ok, lines }
        }
```

- [ ] **Step 5: Bump the protocol**

In `crates/neotmux-proto/src/lib.rs`: `pub const PROTOCOL_VERSION: u32 = 9;`

- [ ] **Step 6: Run the tests**

Run: `cargo test -p neotmux-proto`
Expected: PASS, every test.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux-proto
git commit -m "$(cat <<'EOF'
the wire carries a command and its answer

A connection that sends one is not an attach, which the doorstep already
distinguishes. The answer is lines and whether it worked, because that is
what a command line wants and nothing more.
EOF
)"
```

---

### Task 2: The door reads one, and the CLI asks it

Nothing is implemented yet, so every command name is unknown — and that is the deliverable:
`neotmux frobnicate` says so on stderr and exits 1, through the whole path.

**Files:**
- Create: `crates/neotmux/src/command.rs`
- Modify: `crates/neotmux/src/lib.rs` (or wherever the sibling modules are declared — follow
  what `notify.rs` does)
- Modify: `crates/neotmux/src/server.rs`
- Modify: `crates/neotmux/src/main.rs`
- Test: `crates/neotmux/src/command.rs` (unit), `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: `ToServer::Command`, `ToClient::Ran` from Task 1.
- Produces:
  - `pub struct Ran { pub ok: bool, pub lines: Vec<String> }` with `Ran::ok(lines)` and
    `Ran::failed(sentence)` constructors.
  - `pub const NAMES: &[&str]` — every host command name, for the CLI to recognise.
  - `pub fn run(name: &str, args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran`
    where `Ctx` carries what the commands need from the server (filled in as the commands
    arrive; in this task it carries nothing but the sessions).

- [ ] **Step 1: Write the failing unit test**

In `crates/neotmux/src/command.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_nobody_implements_is_a_failure_that_says_so() {
        let ran = unknown("frobnicate");
        assert!(!ran.ok);
        assert_eq!(ran.lines, vec!["unknown command 'frobnicate'".to_string()]);
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux a_name_nobody_implements 2>&1 | tail -20`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Write the module**

`crates/neotmux/src/command.rs`:

```rust
//! The commands the host owns.
//!
//! These control the server — its sessions, their panes, what is written into one and what is
//! on it — and they are answered without the Lua layer being consulted, entered or woken. That
//! is the whole of why they are here rather than in the runtime layer: a control CLI that
//! stops working over how a config is written is not a control CLI.
//!
//! Nothing in this file knows about the socket. A name, its arguments and the sessions go in;
//! lines and whether it worked come out.

/// What a command did.
pub struct Ran {
    pub ok: bool,
    /// stdout where `ok`, and the one sentence that says why not where it is not.
    pub lines: Vec<String>,
}

impl Ran {
    pub fn ok(lines: Vec<String>) -> Self {
        Self { ok: true, lines }
    }

    pub fn did(line: impl Into<String>) -> Self {
        Self::ok(vec![line.into()])
    }

    pub fn failed(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            lines: vec![why.into()],
        }
    }
}

/// Every name the host answers. The CLI reads it to tell a command from a session name.
pub const NAMES: &[&str] = &[];

pub fn unknown(name: &str) -> Ran {
    Ran::failed(format!("unknown command '{name}'"))
}
```

Declare it beside the other modules (`mod command;` in `crates/neotmux/src/main.rs` if that is
where `mod notify;` lives — check and follow).

- [ ] **Step 4: Run the unit test**

Run: `cargo test -p neotmux a_name_nobody_implements`
Expected: PASS.

- [ ] **Step 5: Teach the door the message**

In `server.rs`, in `enum Ask`:

```rust
enum Ask {
    List,
    Kill { session: String },
    Command { session: Option<String>, name: String, args: Vec<String> },
}
```

In `doorstep`, beside the `List` and `Kill` arms:

```rust
            Ok(ToServer::Command {
                session,
                name,
                args,
            }) => Doorstep::Asking(Ask::Command {
                session,
                name,
                args,
            }),
```

In `answer`, a third arm:

```rust
        Ask::Command { session, name, args } => {
            let ran = crate::command::unknown(&name);
            let _ = link.send(&wire::encode_to_client(&ToClient::Ran {
                ok: ran.ok,
                lines: &ran.lines,
            }));
        }
```

(Task 3 replaces the body of that arm with the real dispatch; the shape is what this task
delivers.)

- [ ] **Step 6: Teach the CLI to ask**

In `main.rs`: add `Action::Command { name: String, args: Vec<String> }` to the `Action` enum,
and in `parse_args`, where a bare word becomes the session name today, check
`command::NAMES.contains(&arg)` first — a word that names a command is a command, and
everything after it is that command's argument, flags included. `-t <name>` is read there too,
into `Cli::session`.

Then:

```rust
/// Runs a host command and prints what it said. Lines on stdout where it worked, the one
/// sentence on stderr where it did not, and the exit status says which.
fn run_command(name: &str, args: &[String], session: Option<&str>) -> anyhow::Result<()> {
    let asked = ask(&neotmux_proto::msg::ToServer::Command {
        session: session.map(str::to_string),
        name: name.to_string(),
        args: args.to_vec(),
    })?;
    match asked {
        Some(neotmux_proto::OwnedToClient::Ran { ok: true, lines }) => {
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Some(neotmux_proto::OwnedToClient::Ran { ok: false, lines }) => {
            anyhow::bail!("{}", lines.join("\n"))
        }
        Some(other) => anyhow::bail!("the neotmux server answered {other:?}"),
        None => anyhow::bail!("no neotmux server is running"),
    }
}
```

and dispatch it from the `match cli.action` block.

- [ ] **Step 7: Write the socket test**

In `crates/neotmux/tests/socket.rs`:

```rust
/// The whole path, with nothing implemented at the end of it: a name the host does not know
/// is a sentence on stderr and a non-zero exit, not a silent success.
#[test]
fn a_command_nobody_implements_fails_and_says_which_name() {
    let runtime = TestRuntime::new();
    let client = started(runtime.path());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("frobnicate")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run frobnicate");
    drop(client);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("unknown command 'frobnicate'"), "{said}");
}
```

Note: `frobnicate` is not in `NAMES`, so `parse_args` reads it as a session name. Give the test
a name that *is* in `NAMES` once Task 3 fills the table — for this task, send the message
directly with `ToServer::Command` over a `UnixStream` the way the protocol tests in this file
already do, and keep the CLI assertion for Task 3.

- [ ] **Step 8: Run the suite**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/neotmux
git commit -m "$(cat <<'EOF'
the door answers a command

The arm, the module the commands will live in, and the one path from the
command line to the server and back: lines on stdout, the sentence on
stderr, the exit status saying which. Every name is unknown so far.
EOF
)"
```

---

### Task 3: `new` and `rename`

**Files:**
- Modify: `crates/neotmux/src/command.rs`
- Modify: `crates/neotmux/src/server.rs`
- Modify: `crates/neotmux/src/main.rs` (usage text, and `new` starting a server)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: `Ran`, `NAMES`, `unknown` from Task 2.
- Produces: `pub struct Ctx<'a> { pub sessions: &'a mut Sessions, pub plugins: &'a Plugins,
  pub terminals: &'a Rc<PaneTerminals>, pub waker: &'a Waker }` and
  `pub fn run(name: &str, args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran`.
  Later tasks add arms to `run` and nothing else.

`Sessions`, `Held`, `setup_for` and `Session::new` are private to `server.rs` today. Make
`Sessions`, `Held`, `mint`, `id_of`, `names` and `get_mut` `pub(crate)`, and move nothing:
`command.rs` is a reader of the server's own types, not a new home for them.

**Every one of these commands already exists as a `ServerOp`.** `apply_server_ops` in
`server.rs` handles `ServerOp::{Create, Switch, Kill, Rename}` for a plugin that asked. Do not
write a second implementation of any of them: lift each arm's body into a free function in
`server.rs` that both callers use — `pub(crate) fn start_session(name: &str, size: Size, …) ->
Result<(), String>` and `pub(crate) fn rename_session(from: &str, to: &str, …) -> Result<(),
String>` — with the plugin's arm logging the `Err` the way it does now and the command turning
it into `Ran::failed`. The difference between the two callers is where the failure goes, and
that is the only difference there should be.

- [ ] **Step 1: Write the failing socket test**

```rust
/// A session with no client: what an agent starts before it has anywhere to look.
#[test]
fn new_starts_a_session_nobody_is_attached_to_and_ls_names_it() {
    let runtime = TestRuntime::new();
    let client = started(runtime.path());

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["new", "agent-7"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run new");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let listed = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("ls")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run ls");
    drop(client);
    let names = String::from_utf8_lossy(&listed.stdout);
    assert!(names.contains("agent-7"), "{names}");
}

/// The name is a word, and the same law the command line and the door already enforce.
#[test]
fn new_refuses_a_name_that_is_not_a_word() {
    let runtime = TestRuntime::new();
    let client = started(runtime.path());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["new", "../escape"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run new");
    drop(client);
    assert!(!out.status.success());
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux --test socket new_ 2>&1 | tail -20`
Expected: FAIL — `unknown command 'new'`.

- [ ] **Step 3: Implement the two commands**

In `command.rs`:

```rust
pub const NAMES: &[&str] = &["new", "rename"];

pub fn run(name: &str, args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    match name {
        "new" => new(args, session, ctx),
        "rename" => rename(args, ctx),
        other => unknown(other),
    }
}

/// A session started with no client attached. Its surface is 80x24 because a surface has to
/// be some size before a shell can open on it, and the first client to attach refits it —
/// the `window-size latest` rule sessions already follow.
const HEADLESS: Size = Size { rows: 24, cols: 80 };

fn new(args: &[String], session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let name = args
        .first()
        .map(String::as_str)
        .or(session)
        .unwrap_or(neotmux::sock::DEFAULT_SESSION)
        .to_string();
    if let Err(why) = neotmux::sock::validate_session_name(&name) {
        return Ran::failed(why);
    }
    if ctx.sessions.id_of(&name).is_some() {
        return Ran::failed(format!("a session called '{name}' is already running"));
    }
    let made = Session::new(
        &setup_for(ctx.plugins, HEADLESS),
        ctx.waker.clone(),
        ctx.terminals.clone(),
        ctx.sessions.panes.clone(),
    );
    match made {
        Ok(session) => {
            let id = ctx.sessions.mint();
            ctx.sessions.live.insert(id, Held { name: name.clone(), session });
            ctx.plugins.lua().name_session(id, &name);
            Ran::did(name)
        }
        Err(err) => Ran::failed(format!("could not start '{name}': {err:#}")),
    }
}

fn rename(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let [from, to] = args else {
        return Ran::failed("usage: neotmux rename <old> <new>");
    };
    if let Err(why) = neotmux::sock::validate_session_name(to) {
        return Ran::failed(why);
    }
    let Some(id) = ctx.sessions.id_of(from) else {
        return Ran::failed(format!("neotmux session '{from}' is not running"));
    };
    if ctx.sessions.id_of(to).is_some() {
        return Ran::failed(format!("a session called '{to}' is already running"));
    }
    if let Some(held) = ctx.sessions.live.get_mut(&id) {
        held.name = to.clone();
    }
    ctx.plugins.lua().name_session(id, to);
    Ran::did(to.clone())
}
```

`name_session` is what the Lua layer reads a session's name from; renaming without it leaves
`neotmux.session.list()` saying the old one.

- [ ] **Step 4: Wire it into `answer`**

`answer` needs the terminals and the waker, which `Serving` holds. Pass `serving` into
`answer` — `door` already has it — and build the `Ctx` there:

```rust
        Ask::Command { session, name, args } => {
            let mut ctx = crate::command::Ctx {
                sessions,
                plugins,
                terminals: serving.terminals,
                waker: serving.waker,
            };
            let ran = crate::command::run(&name, &args, session.as_deref(), &mut ctx);
            let _ = link.send(&wire::encode_to_client(&ToClient::Ran {
                ok: ran.ok,
                lines: &ran.lines,
            }));
        }
```

- [ ] **Step 5: Let `new` start a server**

`ask` returns `None` when nothing is listening, which is the right answer for `ls` and wrong
for `new`. In `main.rs`, give `new` the client's own path: `start::begin()?.finish()?` starts a
server if there is none, then send the `Command` on that link and read the `Ran`. Keep `ask`
for every other command.

- [ ] **Step 6: Update the usage text**

In `usage()`, under `commands:`:

```
  neotmux new [name]  start a session with no client attached
  neotmux rename <old> <new>
                      rename a session
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p neotmux --test socket && cargo test --workspace`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/neotmux
git commit -m "$(cat <<'EOF'
a session can be started without a client

`neotmux new agent-7` is the first half of an agent using neotmux: a session
to work in, laid out at 80x24 because nobody is looking at it yet, refitted
by whichever client attaches first. `rename` comes with it, because a name
the server holds is a name something has to be able to change.
EOF
)"
```

---

### Task 4: `windows` and `panes`

**Files:**
- Modify: `crates/neotmux/src/session.rs` (two accessors)
- Modify: `crates/neotmux/src/command.rs`
- Modify: `crates/neotmux/src/main.rs` (usage text)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: `Ctx`, `Ran`, `run` from Task 3.
- Produces: `Session::windows_list(&self) -> Vec<(WindowId, Vec<PaneId>, Option<String>)>`
  — `Windows::named_panes()`'s own shape, which `Session::geometry` already maps from — and
  `Session::pane_rows(&self) -> Vec<PaneRow>` where
  `pub struct PaneRow { pub pane: PaneId, pub window: WindowId, pub command: Option<String>,
  pub cwd: Option<String> }`.

- [ ] **Step 1: Write the failing socket test**

```rust
/// What an agent asks before it sends anything: which pane is which.
#[test]
fn panes_names_the_pane_and_what_it_is_running() {
    let runtime = TestRuntime::new();
    let client = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("panes")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run panes");
    drop(client);
    let rows = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(rows.starts_with("%1 @1 "), "{rows}");
    assert!(rows.contains("cat"), "{rows}");
}

#[test]
fn windows_names_the_window_and_counts_its_panes() {
    let runtime = TestRuntime::new();
    let client = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("windows")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run windows");
    drop(client);
    assert!(out.status.success());
    let rows = String::from_utf8_lossy(&out.stdout);
    assert!(rows.contains("@1"), "{rows}");
    assert!(rows.contains("panes=1"), "{rows}");
}

#[test]
fn panes_of_a_session_that_is_not_running_fails() {
    let runtime = TestRuntime::new();
    let client = started(runtime.path());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["panes", "-t", "nowhere"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run panes");
    drop(client);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("neotmux session 'nowhere' is not running"), "{said}");
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux --test socket panes_ 2>&1 | tail -20`
Expected: FAIL — `unknown command 'panes'`.

- [ ] **Step 3: Add the accessors**

In `session.rs`:

```rust
/// One pane, as a command that lists them says it. The size is not here: it is the
/// terminal's, which is the whole server's rather than one session's.
pub struct PaneRow {
    pub pane: PaneId,
    pub window: WindowId,
    pub command: Option<String>,
    pub cwd: Option<String>,
}

impl Session {
    /// Every window, in creation order, with the panes it holds and whatever it is called.
    /// `Windows::named_panes`'s own tuples: a command prints them, and the proto's `Window`
    /// is the frame's shape, not this one's.
    pub fn windows_list(&self) -> Vec<(WindowId, Vec<PaneId>, Option<String>)> {
        self.windows.named_panes()
    }

    /// Every pane the session holds, hidden windows included, with what its shell is running
    /// read now rather than off the frame's cadence: a command answers a question asked at
    /// this instant, and the cache exists for a thing drawn sixty times a second.
    pub fn pane_rows(&self) -> Vec<PaneRow> {
        let mut rows = Vec::new();
        for (window, panes, _name) in self.windows.named_panes() {
            for pane in panes {
                let info = match self.shells.foreground(pane) {
                    Some(pid) => crate::procinfo::read(pid),
                    None => PaneInfo::default(),
                };
                rows.push(PaneRow {
                    pane,
                    window,
                    command: info.command,
                    cwd: info.cwd,
                });
            }
        }
        rows
    }
}
```

- [ ] **Step 4: Implement the two commands**

In `command.rs`, add `"windows"` and `"panes"` to `NAMES`, arms to `run`, and:

```rust
/// The session a command is addressed to, or the sentence that says there is none.
fn addressed<'a>(session: Option<&str>, ctx: &'a mut Ctx<'_>) -> Result<&'a mut Session, Ran> {
    let name = session.unwrap_or(neotmux::sock::DEFAULT_SESSION);
    match ctx.sessions.id_of(name) {
        Some(id) => Ok(ctx.sessions.get_mut(id).expect("a session it just named")),
        None => Err(Ran::failed(format!("neotmux session '{name}' is not running"))),
    }
}

fn windows(session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let held = match addressed(session, ctx) {
        Ok(held) => held,
        Err(no) => return no,
    };
    Ran::ok(
        held.windows_list()
            .into_iter()
            .map(|(id, panes, name)| {
                format!(
                    "@{} {} panes={}",
                    id.get(),
                    name.as_deref().unwrap_or("-"),
                    panes.len()
                )
            })
            .collect(),
    )
}

fn panes(session: Option<&str>, ctx: &mut Ctx<'_>) -> Ran {
    let terminals = ctx.terminals.clone();
    let held = match addressed(session, ctx) {
        Ok(held) => held,
        Err(no) => return no,
    };
    Ran::ok(
        held.pane_rows()
            .into_iter()
            .map(|row| {
                let size = terminals
                    .size(row.pane)
                    .map(|size| format!("{}x{}", size.cols, size.rows))
                    .unwrap_or_else(|| "-".to_string());
                format!(
                    "%{} @{} {} {} {}",
                    row.pane.get(),
                    row.window.get(),
                    size,
                    row.command.as_deref().unwrap_or("-"),
                    row.cwd.as_deref().unwrap_or("-"),
                )
            })
            .collect(),
    )
}
```

- [ ] **Step 5: Update the usage text**

```
  neotmux windows [-t name]
                      the session's windows, one per line
  neotmux panes [-t name]
                      the session's panes, one per line
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p neotmux --test socket && cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux
git commit -m "$(cat <<'EOF'
the panes and windows of a session can be asked for

An agent that did not open the session has no way to know what is in it.
Now it asks: a line per pane saying which window holds it, how big it is
and what its shell is running, read at the instant of the question rather
than off the frame's cadence.
EOF
)"
```

---

### Task 5: `send` and `capture`

**Files:**
- Modify: `crates/neotmux/src/session.rs` (one accessor)
- Modify: `crates/neotmux/src/command.rs`
- Modify: `crates/neotmux/src/main.rs` (usage text)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: `Ctx`, `Ran`, `addressed` from Task 4.
- Produces: `Session::write_to_pane(&mut self, pane: PaneId, bytes: &[u8]) ->
  anyhow::Result<bool>` — `false` where the session does not hold that pane.

- [ ] **Step 1: Write the failing socket test**

```rust
/// The row of the herdr table, in one test: an agent writes into a pane it did not open and
/// reads back what the shell printed.
#[test]
fn send_reaches_the_pane_and_capture_reads_it_back() {
    let runtime = TestRuntime::new();
    let client = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));

    let sent = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["send", "%1", "from-outside", "--enter"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run send");
    assert!(sent.status.success(), "{}", String::from_utf8_lossy(&sent.stderr));

    let deadline = Instant::now() + PATIENCE;
    let seen = loop {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
            .args(["capture", "%1"])
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("HOME", runtime.path())
            .output()
            .expect("run capture");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.contains("from-outside") {
            break text;
        }
        assert!(Instant::now() < deadline, "capture never saw it: {text:?}");
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(client);
    assert!(seen.contains("from-outside"), "{seen}");
}

#[test]
fn send_to_a_pane_that_is_not_there_fails() {
    let runtime = TestRuntime::new();
    let client = started(runtime.path());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["send", "%99", "nothing"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .output()
        .expect("run send");
    drop(client);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("no pane %99"), "{said}");
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux --test socket send_ 2>&1 | tail -20`
Expected: FAIL — `unknown command 'send'`.

- [ ] **Step 3: Add the accessor**

In `session.rs`:

```rust
    /// Bytes into a pane's shell, as though they had been typed at it. `false` is a pane this
    /// session does not hold, which is the caller's cue to ask another one: pane ids are the
    /// server's, not a session's.
    pub fn write_to_pane(&mut self, pane: PaneId, bytes: &[u8]) -> anyhow::Result<bool> {
        if !self.windows.all_panes().any(|held| held == pane) {
            return Ok(false);
        }
        self.shells.write(pane, bytes)?;
        Ok(true)
    }
```

- [ ] **Step 4: Implement the two commands**

In `command.rs`, add `"send"` and `"capture"` to `NAMES`, arms to `run`, and:

```rust
/// `%4`, the way a pane is written everywhere else. A bare number is taken too: a caller that
/// has the id out of a `panes` line should not have to remember the sigil.
fn pane_id(word: &str) -> Option<PaneId> {
    word.trim_start_matches('%').parse::<u32>().ok().map(PaneId::from)
}

fn send(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let Some((first, rest)) = args.split_first() else {
        return Ran::failed("usage: neotmux send <pane> <text> [--enter]");
    };
    let Some(pane) = pane_id(first) else {
        return Ran::failed(format!("'{first}' is not a pane id"));
    };
    let enter = rest.iter().any(|arg| arg == "--enter");
    let text = rest
        .iter()
        .filter(|arg| *arg != "--enter")
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let mut bytes = text.into_bytes();
    if enter {
        bytes.push(b'\r');
    }
    // Pane ids are the server's, so the pane is looked for in every session rather than in the
    // one `-t` named — which is why `send` takes no `-t` at all.
    for id in ctx.sessions.ids() {
        let Some(session) = ctx.sessions.get_mut(id) else {
            continue;
        };
        match session.write_to_pane(pane, &bytes) {
            Ok(true) => return Ran::ok(Vec::new()),
            Ok(false) => continue,
            Err(err) => return Ran::failed(format!("could not write to %{}: {err:#}", pane.get())),
        }
    }
    Ran::failed(format!("no pane %{}", pane.get()))
}

fn capture(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let Some(first) = args.first() else {
        return Ran::failed("usage: neotmux capture <pane> [--lines N]");
    };
    let Some(pane) = pane_id(first) else {
        return Ran::failed(format!("'{first}' is not a pane id"));
    };
    let back = match args.iter().position(|arg| arg == "--lines") {
        Some(at) => match args.get(at + 1).and_then(|n| n.parse::<usize>().ok()) {
            Some(n) => Some(n),
            None => return Ran::failed("--lines takes a number"),
        },
        None => None,
    };
    let want = Want::default();
    let Some(visible) = ctx.terminals.content(pane, None, want) else {
        return Ran::failed(format!("no pane %{}", pane.get()));
    };
    let Some(count) = back else {
        return Ran::ok(visible.lines);
    };
    // `--lines N` is the last N lines of everything the pane has: its history, then what is on
    // it now. A pane that has scrolled nothing answers with what it has.
    let extent = ctx.terminals.history_extent(pane).unwrap_or_default();
    let wanted_history = count.saturating_sub(visible.lines.len());
    let from = usize::try_from(extent.len).unwrap_or(usize::MAX).saturating_sub(wanted_history);
    let mut lines = match ctx.terminals.history(pane, from, wanted_history, want) {
        Some(history) => history.lines,
        None => Vec::new(),
    };
    lines.extend(visible.lines);
    let cut = lines.len().saturating_sub(count);
    Ran::ok(lines.split_off(cut))
}
```

`Sessions::ids()` is a new one-liner beside `names()`:

```rust
    pub(crate) fn ids(&self) -> Vec<SessionId> {
        self.live.keys().copied().collect()
    }
```

- [ ] **Step 5: Update the usage text**

```
  neotmux send <pane> <text> [--enter]
                      write text into that pane's shell
  neotmux capture <pane> [--lines N]
                      what that pane is showing, oldest line first
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p neotmux --test socket && cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux
git commit -m "$(cat <<'EOF'
a pane can be written to and read from the outside

The row of the herdr table this was all for: something that is not a person
at a terminal starts a session, types into it and reads what came back, over
the socket and without a key ever being simulated.
EOF
)"
```

---

### Task 6: `switch`

**Files:**
- Modify: `crates/neotmux/src/command.rs`
- Modify: `crates/neotmux/src/server.rs` (the clients are the door's, not a session's)
- Modify: `crates/neotmux/src/main.rs` (usage text)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Consumes: everything from Task 5.
- Produces: `Ctx` gains `pub clients: &'a mut Vec<Attached>`; every earlier command ignores it.

Switching is what `ServerOp::Switch` already does for a plugin — read that arm in
`apply_server_ops` and do the same thing here rather than inventing a second way to move a
client. The one difference is that a plugin's switch moves the client that asked, and this one
moves every client attached to this server, which is what "bring me to that session" means from
outside.

- [ ] **Step 1: Write the failing socket test**

```rust
/// A client looking at one session, moved to another by something that is not that client.
#[test]
fn switch_moves_the_attached_client_to_the_named_session() {
    let runtime = TestRuntime::new();
    let client = Client::start(runtime.path(), "/bin/cat");
    wait_for_socket(&socket_in(runtime.path()));
    client.send("in-the-first\r");
    client.wait_for_output("in-the-first");

    for args in [vec!["new", "second"], vec!["switch", "second"]] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
            .args(&args)
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("HOME", runtime.path())
            .output()
            .expect("run a command");
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    client.send("in-the-second\r");
    client.wait_for_output("in-the-second");
    drop(client);
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux --test socket switch_ 2>&1 | tail -20`
Expected: FAIL — `unknown command 'switch'`.

- [ ] **Step 3: Implement it**

Add `"switch"` to `NAMES` and an arm to `run`:

```rust
fn switch(args: &[String], ctx: &mut Ctx<'_>) -> Ran {
    let Some(name) = args.first() else {
        return Ran::failed("usage: neotmux switch <name>");
    };
    let Some(id) = ctx.sessions.id_of(name) else {
        return Ran::failed(format!("neotmux session '{name}' is not running"));
    };
    let moved = crate::server::switch_clients(id, None, ctx.clients, ctx.sessions, ctx.plugins);
    if moved == 0 {
        return Ran::failed(format!("nothing is attached to move to '{name}'"));
    }
    Ran::did(name.clone())
}
```

`ServerOp::Switch` in `apply_server_ops` already does this for a plugin: it moves the clients,
then calls `session.adopt(&setup_for(plugins, size))` with the arriving client's size, because
a session laid out for the previous client is the bug. Lift that body into
`pub(crate) fn switch_clients(to: SessionId, of: Option<SessionId>, …)` — `of` being the
session whose clients move, `None` meaning every client — and call it from both places. The
plugin passes the session that asked; the command passes `None`.

- [ ] **Step 4: Update the usage text**

```
  neotmux switch <name>
                      move the attached clients to that session
```

- [ ] **Step 5: Run the whole suite**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Update the spec's status**

In `docs/superpowers/specs/2026-09-23-the-command-surface-design.md` §9, mark stages 1–3 done
with today's date and name this plan, the way the other specs in that directory record what
has landed.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux docs
git commit -m "$(cat <<'EOF'
a client can be moved between sessions from outside

The last of the host commands: `neotmux switch work` brings whoever is
attached to the session you name, which is what herdr's "take me to that
workspace" is when the thing asking is not the client.
EOF
)"
```

---

## Self-Review

**Spec coverage:** §3's nine commands — `ls` and `kill` exist already; `new`/`rename` are Task
3, `windows`/`panes` Task 4, `send`/`capture` Task 5, `switch` Task 6. §6's "answered at the
door, no frame entered" is Task 2's `answer` arm and holds through every later task. §7's
messages are Task 1. §8's tests: the live-server ones are in Tasks 3–6, the wire round-trip in
Task 1, the refusals in Tasks 3, 4 and 5. §2's line is the Global Constraints' first and second
bullets.

**Not covered, and deliberately:** §4, §5 and §8's name-resolution and ambient-binding tests
are stage 4 — `neotmux.command` and Lua dispatch — which is its own plan. Nothing in this plan
registers, resolves or falls back to anything: every name here is the host's, and the
resolution order only becomes a question when a second namespace exists.

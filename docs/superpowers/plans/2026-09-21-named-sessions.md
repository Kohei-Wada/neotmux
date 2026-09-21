# Named Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `neotmux [name]` starts or attaches a named session, `ls` lists the running ones, `kill [name]` ends one of them, and `neotmux.session_name()` tells a plugin which one it is in.

**Architecture:** The socket layout is already one file per session (`$XDG_RUNTIME_DIR/neotmux/<name>.sock`); this plan threads a name through `sock::Paths` and the CLI, and `ls` is a connect-probe scan of that directory, not a protocol message. The session the server holds and the frame loop are untouched — this is the M4a stage of the sessions spec, and the M4b stage (two clients on one session) lands later and builds on the harness this adds.

**Tech Stack:** Rust 2024 (workspace `rust-version = "1.95"`), `nix` (already present, `poll`/`fcntl`/`term` features), `anyhow`, `mlua` (vendored Lua 5.4), `tempfile`/`portable-pty` (test). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-21-sessions-design.md` (§3–§5 are this plan; §6–§7 are the later `2026-09-21-two-clients.md`)

## Global Constraints

- Session names: `[A-Za-z0-9_-]`, 1–64 characters, **first character not `-`** (spec §3's law, tightened: the `--server` and `kill` paths both parse an optional following argument, and a leading-dash name would make `--server -work` indistinguishable from a flag). `default` is unchanged and valid.
- `--kill` is removed from the CLI; `kill [name]` replaces it. `CHANGELOG.md` and the README's status section say so.
- `ls` takes no name; a name given with `ls` is refused.
- Refusing an invalid name happens at argument parsing, before any file exists (spec §3).
- One session name per process: there is no `--session` flag; the name is positional.

## Review Focus

- `kill` with a name that is not running must refuse and must **not** fall back to killing `default`.
- A socket file left by a dead server (refused connect) must not appear in `ls`; a socket that answers must.
- Two `neotmux work` starting at the same instant end with exactly one server.
- `ls` output is stable and ordered, so a person scripting on it is not surprised.
- The existing `default` flow (start, attach again, `C-b d` detach, kill) is byte-identical in behaviour for anything not named.

---

### Task 1: Names in the socket layer

**Files:**
- Modify: `crates/neotmux/src/sock.rs`
- Modify: `crates/neotmux/tests/session/mod.rs:14` (`end_session` calls `Paths::resolve`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `sock::validate_session_name(name: &str) -> Result<(), String>`, `sock::Paths::resolve(env: &dyn Fn(&str) -> Option<OsString>, name: &str) -> anyhow::Result<Paths>`, `sock::DEFAULT_SESSION: &str` (the existing const, made `pub`), `sock::socket_dir(env) -> anyhow::Result<PathBuf>` (pub wrapper over the private `runtime_dir`).

- [ ] **Step 1: Write the failing test**

In `crates/neotmux/src/sock.rs`, in the existing `mod tests` (it already has a `resolve` helper test fixture around line 227):

```rust
    #[test]
    fn a_name_is_a_lawful_filename_and_nothing_more() {
        for name in ["work", "default", "a", "a-b_c9", &"x".repeat(64)] {
            assert!(validate_session_name(name).is_ok(), "{name}");
        }
        for name in ["", &"x".repeat(65), "a:b", "a/b", "a b", "-lead", "a..b"] {
            assert!(validate_session_name(name).is_err(), "{name} should be refused");
        }
    }

    #[test]
    fn resolve_builds_paths_for_the_name_asked_for() {
        let dir = PathBuf::from("/rt");
        let paths =
            Paths::resolve(&env_for(dir.clone()), "work").expect("resolve");
        assert_eq!(paths.socket, dir.join("neotmux/work.sock"));
        assert_eq!(paths.lock, dir.join("neotmux/work.lock"));
    }
```

Use the test file's existing env-closure fixture for the `XDG_RUNTIME_DIR` value (the existing tests build one the same way; if the fixture is named differently there, use that name — it closes over an absolute path).

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p neotmux --lib sock::`
Expected: FAIL — `validate_session_name` and the two-argument `resolve` do not exist.

- [ ] **Step 3: Implement**

In `crates/neotmux/src/sock.rs`:

```rust
/// The only default there is. A client that was not told a name is holding this one.
pub const DEFAULT_SESSION: &str = "default";

/// What a session name may be. It becomes a filename, so the law is the filesystem's
/// and the argv's at once: one to sixty-four of `[A-Za-z0-9_-]`, and the first is not
/// `-`, because an argument that looks like a flag is not a name the parser can tell it
/// apart from one.
pub fn validate_session_name(name: &str) -> Result<(), String> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(format!("session name '{name}' is not one to sixty-four characters long"));
    }
    if bytes[0] == b'-' {
        return Err(format!("session name '{name}' may not start with '-'"));
    }
    if !bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_') {
        return Err(
            format!("session name '{name}' may only use letters, digits, '-' and '_'"),
        );
    }
    Ok(())
}
```

Change the const and `resolve`:

```rust
    pub fn resolve(
        env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
        name: &str,
    ) -> anyhow::Result<Self> {
        let dir = runtime_dir(env);
        make_dir(&dir).with_context(|| format!("prepare {}", dir.display()))?;
        Ok(Self {
            socket: dir.join(format!("{name}.sock")),
            lock: dir.join(format!("{name}.lock")),
        })
    }
```

Expose the directory for `ls` (in the same module, next to `runtime_dir`):

```rust
/// Where a session's files live, whatever the name is. `ls` needs the directory rather
/// than one name.
pub fn socket_dir(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    runtime_dir(env)
}
```

Update the module doc line 24–25 comment on `DEFAULT_SESSION` (it says "until more than one session is a thing" — now they are).
Update `crates/neotmux/tests/session/mod.rs:14`:

```rust
    let Ok(paths) = neotmux::sock::Paths::resolve(
        &move |key| (key == "XDG_RUNTIME_DIR").then(|| runtime_dir.clone().into_os_string())),
        neotmux::sock::DEFAULT_SESSION,
    ) else {
```

- [ ] **Step 4: Run the tests and the workspace build**

Run: `cargo test -p neotmux --lib sock:: && cargo build --workspace`
Expected: PASS; the build fails only if some other caller of `Paths::resolve` was missed — fix each with `neotmux::sock::DEFAULT_SESSION` (grep `Paths::resolve` for them).

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/src/sock.rs crates/neotmux/tests/session/mod.rs
git commit -m "thread the session name through the socket's file names"
```

---

### Task 2: The CLI surface

**Files:**
- Modify: `crates/neotmux/src/main.rs` (the `Cli` struct, `parse_args`, `usage`, `kill`, `main`; its `mod tests`)
- Modify: `crates/neotmux/tests/socket.rs` — the two existing `neotmux --kill` invocations (`kill_ends_a_session_nobody_is_attached_to` around line 651, `kill_with_nothing_running_says_so_and_fails` around line 778) become `kill` with no name: the same session, the same expectations, the new spelling

**Interfaces:**
- Consumes: `sock::validate_session_name`, `sock::Paths::resolve(env, name)`, `sock::socket_dir(env)`, `sock::holder`, `sock::connect`.
- Produces: `Cli { log, action: Action, session: Option<String>, server, help, version, refusal: Option<String>, warnings }`, `enum Action { Attach, Ls, Kill }`, `kill_session(name: &str) -> anyhow::Result<()>`, `list_sessions() -> anyhow::Result<()>`. Task 3 builds on `cli.session` and `cli.action`.

- [ ] **Step 1: Write the failing tests**

In `crates/neotmux/src/main.rs` `mod tests` (the existing tests' `args` helper stays; the existing `the_kill_flag_is_recognised_and_is_not_a_warning` test is **replaced**, not kept — `--kill` is gone):

```rust
    #[test]
    fn an_unnamed_attach_is_still_the_default_and_warns_on_nothing() {
        let cli = parse_args(args(&[]));
        assert!(matches!(cli.action, Action::Attach));
        assert!(cli.session.is_none());
        assert!(cli.session_name().unwrap() == "default");
        assert!(cli.warnings.is_empty());
    }

    #[test]
    fn one_positional_is_the_session_it_attaches() {
        let cli = parse_args(args(&["work"]));
        assert!(matches!(cli.action, Action::Attach));
        assert_eq!(cli.session.as_deref(), Some("work"));
        assert!(cli.warnings.is_empty());
    }

    #[test]
    fn ls_is_a_thing_and_takes_no_name() {
        let cli = parse_args(args(&["ls"]));
        assert!(matches!(cli.action, Action::Ls));
        assert!(cli.refusal.is_none());

        let cli = parse_args(args(&["ls", "work"]));
        assert!(cli.refusal.is_some(), "a name with ls is a mistake, not a filter");
    }

    #[test]
    fn kill_is_a_thing_and_the_name_is_optional() {
        let cli = parse_args(args(&["kill"]));
        assert!(matches!(cli.action, Action::Kill));
        assert!(cli.session.is_none(), "no name is the default session");
        assert!(cli.refusal.is_none());

        let cli = parse_args(args(&["kill", "work"]));
        assert_eq!(cli.session.as_deref(), Some("work"));
        assert!(cli.refusal.is_none());
    }

    #[test]
    fn an_invalid_name_is_refused_and_said_so_not_a_warning() {
        // A refused name is a refusal: the command is not run. In particular `kill` with
        // a name it could not confirm must not fall back to `default`, which is the one
        // case the Review Focus says is dangerous.
        for case in [["kill", "a:b"], ["kill", "-lead"], ["a:b"], ["kill", ""]] {
            let cli = parse_args(args(&case));
            assert!(cli.refusal.is_some(), "{:?} should be refused", case);
            assert!(
                !cli.warnings.iter().any(|w| w.contains("unrecognised")),
                "{:?} was filed under warnings, where it would be ignored: {:?}",
                case, cli.warnings
            );
        }
    }

    #[test]
    fn two_positionals_are_one_name_and_one_misplaced_word() {
        let cli = parse_args(args(&["work", "home"]));
        assert_eq!(cli.session.as_deref(), Some("work"));
        assert_eq!(cli.warnings.len(), 1);
        assert!(cli.warnings[0].contains("home"), "{:?}", cli.warnings);
    }

    #[test]
    fn the_old_kill_flag_is_not_a_flag_anymore() {
        let cli = parse_args(args(&["--kill"]));
        assert!(
            !cli.warnings.iter().any(|w| w.contains("--kill") && w.contains("unrecognised"))
                || cli.refusal.is_some(),
            "'--kill' must be either refused or named unrecognised: {:?}",
            cli.warnings
        );
        assert!(!matches!(cli.action, Action::Kill), "--kill is not the new kill");
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p neotmux --lib main::`
Expected: FAIL — `Action`, `refusal` and the new parse do not exist.

- [ ] **Step 3: Implement the parser**

In `crates/neotmux/src/main.rs`:

```rust
/// What the invocation asks for, before the name. A name is not a mode.
enum Action {
    /// Attach to a session, starting it where it is not running. The mode with no word.
    Attach,
    /// The running sessions, one per line.
    Ls,
    /// End one of them and its shells.
    Kill,
}

struct Cli {
    log: Option<PathBuf>,
    action: Action,
    /// The session the action speaks about, or the default where none was said.
    session: Option<String>,
    server: bool,
    help: bool,
    version: bool,
    /// An argument that is a refusal, not a warning: a name that would not be the
    /// session the person meant to name. Warnings are ignored and gone; this is said
    /// and the command is not run.
    refusal: Option<String>,
    warnings: Vec<String>,
}

impl Cli {
    /// The name as the command will run under, default taken.
    fn session_name(&self) -> anyhow::Result<&str> {
        self.session
            .as_deref()
            .or(Some(neotmux::sock::DEFAULT_SESSION))
            .ok_or_else(|| anyhow::anyhow!("no session"))
    }
}
```

Replace the body of `parse_args` with:

```rust
fn parse_args(mut args: impl Iterator<Item = String>) -> Cli {
    let mut log = None;
    let mut action = Action::Attach;
    let mut session: Option<String> = None;
    let mut server = false;
    let mut help = false;
    let mut version = false;
    let mut refusal: Option<String> = None;
    let mut warnings = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "ls" => {
                action = Action::Ls;
            }
            "kill" => {
                action = Action::Kill;
                match args.next() {
                    // A name for kill must be a name the person clearly meant. One that
                    // starts with `-` is not one, and kill is the one command where
                    // falling back to `default` is the wrong answer: the Review Focus
                    // says kill must not guess.
                    Some(value) if value.starts_with('-') => {
                        refusal = Some(format!(
                            "a name for kill cannot start with '-' ('{value}') — kill does not guess at the default"
                        ));
                    }
                    Some(value) if !value.is_empty() => {
                        if session.is_some() {
                            warnings.push(
                                "a name following 'kill' was ignored, one was already given".to_string(),
                            );
                        }
                        session = Some(value);
                    }
                    None => {}
                }
            }
            "--server" => {
                server = true;
                // `--server` is the spawn path: its name is this process's own, set by
                // `spawn`, and never a person's typing. A flag-shaped word after it is a
                // parse error, not a name; the default is what was meant.
                if let Some(value) = args.next_if(|v| !v.starts_with('-')) {
                    session = Some(value);
                }
            }
            "--log" => match args.next() {
                Some(value) if !value.starts_with('-') => log = Some(PathBuf::from(value)),
                Some(value) => warnings.push(format!(
                    "--log requires a path argument, got '{value}'; using the default log path"
                )),
                None => warnings.push(
                    "--log requires a path argument; using the default log path".to_string(),
                ),
            },
            "--kill" => refusal = Some(
                "'--kill' was the old spelling; it is 'kill [name]' now — try 'neotmux kill'"
                    .to_string(),
            ),
            "--help" | "-h" => help = true,
            "--version" | "-V" => version = true,
            arg if !arg.starts_with('-') && !arg.is_empty() => {
                if session.is_some() {
                    warnings.push(format!(
                        "unrecognised argument '{arg}' after the session name; ignoring it — try 'neotmux --help'"
                    ));
                } else {
                    session = Some(arg.to_string());
                }
            }
            other => warnings.push(format!(
                "unrecognised argument '{other}'; ignoring it — try 'neotmux --help'"
            )),
        }
    }

    // The name, checked once, after the loop, because a name can arrive from the
    // positional, from `kill`, or from `--server`, and `ls` + a name is a refusal either
    // way it arrived.
    if let Some(name) = &session {
        if let Err(why) = neotmux::sock::validate_session_name(name) {
            refusal = Some(why);
        }
    }
    if matches!(action, Action::Ls) && session.is_some() && refusal.is_none() {
        refusal = Some("ls lists the running sessions; it does not take a name".to_string());
    }

    Cli {
        log,
        action,
        session,
        server,
        help,
        version,
        refusal,
        warnings,
    }
}
```

One behaviour to keep straight: a bad name is a **refusal**, full stop — the command is not run, and `kill` with a name it could not confirm does not fall back to `default`. That is the Review Focus line, and it is why `kill`'s arm above rejects a leading `-` at the parse rather than warning and moving on.

Update `usage()`:

```rust
        "\
{version}
A terminal multiplexer: it keeps your shells running after you disconnect, and brings
you back to them, one at a time or by name.

usage: neotmux [name | command] [options]

With no command, neotmux attaches to the named session (default, where no name is
given) if it is running, and starts it if it is not. The session survives your
disconnecting, by any means.

commands:
  neotmux [name]      attach to the named session, or start it
  neotmux ls          the running sessions, one per line
  neotmux kill [name] end a session and every shell in it

options:
  --log <path>    write the log somewhere other than the default
  --help, -h      this text
  --version, -V   the version, and the protocol number it speaks
  --server        the half that holds a session; started for you, not by you
",
```

Update `main()`:

```rust
    let cli = parse_args(env::args().skip(1));
    for warning in &cli.warnings {
        eprintln!("neotmux: {warning}");
    }
    if cli.help {
        print!("{}", usage());
        return Ok(());
    }
    if cli.version {
        println!("{}", version_line());
        return Ok(());
    }
    if let Some(refusal) = &cli.refusal {
        anyhow::bail!("neotmux: {refusal}");
    }
    if let Err(err) = logging::init(cli.log.as_deref()) {
        eprintln!("neotmux: failed to initialise logging: {err:#}");
    }

    if cli.server {
        return start::serve(cli.session.clone());
    }
    match cli.action {
        Action::Ls => return list_sessions(),
        Action::Kill => {
            return kill_session(
                cli.session
                    .as_deref()
                    .unwrap_or(neotmux::sock::DEFAULT_SESSION),
            );
        }
        Action::Attach => return client(cli.session.clone()),
    }
```

- [ ] **Step 4: Implement `kill_session` and `list_sessions`**

Replace `fn kill()` with:

```rust
/// Ends a session's server from outside, for the case the protocol cannot reach: a
/// client built against another version is refused by the handshake, and the session it
/// cannot speak to would otherwise be unreachable and immortal.
fn kill_session(name: &str) -> anyhow::Result<()> {
    let paths = neotmux::sock::Paths::resolve(&|key| env::var_os(key), name)?;
    let pid = match neotmux::sock::holder(&paths)? {
        neotmux::sock::Holder::Nobody => {
            anyhow::bail!("neotmux session '{name}' is not running")
        }
        neotmux::sock::Holder::Unconfirmed => anyhow::bail!(
            "session '{name}''s lock is held, but its pid could not be confirmed; refusing to signal it"
        ),
        neotmux::sock::Holder::Pid(pid) => pid,
    };
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .with_context(|| format!("signal the neotmux server (pid {pid})"))?;
    println!("neotmux: asked the session '{name}' (pid {pid}) to end");
    Ok(())
}

/// The running sessions, the way the socket directory says they are: one file per
/// session, and a connect probe the difference. A socket that answers is a session;
/// one that does not is a file.
fn list_sessions() -> anyhow::Result<()> {
    use std::fs;
    let dir = neotmux::sock::socket_dir(&|key| env::var_os(key));
    let mut running: Vec<(String, Option<i32>)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else { continue };
            let Some(name) = name.strip_suffix(".sock") else { continue };
            if neotmux::sock::connect(&dir.join(format!("{name}.sock")))?.is_some() {
                let pid = fs::read_to_string(dir.join(format!("{name}.lock")))
                    .ok()
                    .and_then(|text| text.trim().parse::<i32>().ok());
                running.push((name.to_string(), pid));
            }
        }
    }
    running.sort_by(|a, b| a.0.cmp(&b.0));
    if running.is_empty() {
        anyhow::bail!("no neotmux sessions are running");
    }
    for (name, pid) in running {
        match pid {
            Some(pid) => println!("{name}  pid {pid}"),
            None => println!("{name}"),
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p neotmux --lib main:: && cargo clippy -p neotmux --all-targets -- -D warnings && cargo test -p neotmux --test socket kill`
Expected: PASS. The old `the_usage_text_names_no_keybinding` test still passes (usage names no keys), and the two `kill` tests in `tests/socket.rs` pass against the new spelling, because their `.arg("--kill")` now says `.arg("kill")`, and nothing else about them did.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux/src/main.rs crates/neotmux/tests/socket.rs
git commit -m "the CLI learns names, ls, and kill — and says so before running"
```

---

### Task 3: The name across the process boundary

**Files:**
- Modify: `crates/neotmux/src/start.rs` (`serve`, `begin`, `spawn`)
- Modify: `crates/neotmux/src/main.rs:196` (`client` passes the name to the Lua layer)

**Interfaces:**
- Consumes: `Cli.session` (Task 2), `sock::DEFAULT_SESSION`, `LuaSubsystem::build_named` (Task 4 — this step only *calls* it, so Task 4 lands first if the build breaks; land the Lua change below first, then this).
- Produces: `start::serve(session: Option<String>)`, `start::begin(session: Option<String>) -> anyhow::Result<Starting>`.

- [ ] **Step 1: Write the failing test**

There is no process-level unit here: this is the wiring, and its test is Task 4's integration test (`a_named_session_is_started_and_ls_shows_it`), which fails until this task and Task 4 both land. For this task the gate is the build: it must fail on the missing names and then succeed.

- [ ] **Step 2: Implement**

In `crates/neotmux/src/start.rs`:

```rust
pub fn serve(session: Option<String>) -> anyhow::Result<()> {
    let name = session
        .as_deref()
        .unwrap_or(neotmux::sock::DEFAULT_SESSION);
    let paths = Paths::resolve(&|key| std::env::var_os(key), name)?;
    // ... unchanged from here on ...
```

```rust
pub fn begin(session: Option<String>) -> anyhow::Result<Starting> {
    let name = session
        .as_deref()
        .unwrap_or(neotmux::sock::DEFAULT_SESSION);
    let paths = Paths::resolve(&|key| std::env::var_os(key), name)?;
    let connected = sock::connect(&paths.socket)?;
    if connected.is_none() {
        spawn(session)?;
    }
    Ok(Starting { paths, connected })
}
```

```rust
fn spawn(session: Option<String>) -> anyhow::Result<()> {
    // ... unchanged until the argument ...
    command.arg("--server");
    if let Some(name) = session {
        command.arg(name);
    }
    // ... unchanged: --log, stdios, setsid, fork ...
}
```

In `crates/neotmux/src/main.rs`, `client()` takes the name and hands it to both halves:

```rust
fn client(session: Option<String>) -> anyhow::Result<()> {
    let _raw = RawMode::enter();
    let starting = start::begin(session.clone())?;
    // ... layers, panes unchanged ...
    let name = session
        .unwrap_or_else(|| neotmux::sock::DEFAULT_SESSION.to_string());
    let lua = LuaSubsystem::build_named(name, layers, panes.clone())?;
    // ... unchanged ...
}
```

- [ ] **Step 3: Build; the only expected error is the missing `build_named`**

Run: `cargo build -p neotmux`
Expected: if `LuaSubsystem::build_named` does not exist yet (Task 4 unlanded), exactly that error. Land Task 4's `neotmux-lua` half, then:

Run: `cargo build --workspace`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/neotmux/src/start.rs crates/neotmux/src/main.rs
git commit -m "the name crosses to the server, and to the client's Lua side"
```

---

### Task 4: `neotmux.session_name()`

**Files:**
- Modify: `crates/neotmux-lua/src/lib.rs` (`build`, `build_named`)
- Modify: `crates/neotmux-lua/src/api/mod.rs` (`Shared`, the `neotmux` table)
- Test: create `crates/neotmux-lua/tests/session_name.rs`
- Test: create `crates/neotmux/tests/fixtures/lua/plugin/session_name.lua` (fixture), assert in `crates/neotmux/tests/fixtures.rs`

**Interfaces:**
- Consumes: the `Shared` table in `api/mod.rs` (it already carries `home` the same way).
- Produces: `LuaSubsystem::build_named(session_name: impl Into<String>, layers: Layers, panes: Rc<dyn PaneRead>) -> anyhow::Result<Self>`; `neotmux.session_name()` in Lua, answering the name.

- [ ] **Step 1: Write the failing test**

Create `crates/neotmux-lua/tests/session_name.rs`:

```rust
//! `neotmux.session_name()`: the one fact about which session this client is in, which
//! nothing in the Lua layer could otherwise be told.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;

mod support;

/// A client that was started with the name is told it by the API.
#[test]
fn the_named_client_is_told_its_name() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "neotmux.on_tick(function()\n\
         neotmux.log.info('name=' .. neotmux.session_name())\n\
         end)\n",
    )
    .unwrap();
    let sub = LuaSubsystem::build_named(
        "work",
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.dispatch_tick();
    let line = sub
        .drain_ops()
        .into_iter()
        .map(|(_, op)| match op {
            neotmux_lua::ops::Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .find(|t| t.starts_with("name="))
        .expect("the tick logged the name");
    assert_eq!(line, "name=work");
}

#[test]
fn a_client_with_no_name_is_told_the_default() {
    let dir = tempfile::tempdir().unwrap();
    // A name that is not the default raises on load, so a clean load is the assertion:
    // the init body checks the name, and `run_init` is where a raise would surface.
    fs::write(
        dir.path().join("init.lua"),
        "if neotmux.session_name() ~= 'default' then\n  error('named: ' .. neotmux.session_name())\nend\n",
    )
    .unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty(), "the layer loads only if the name is the default");
```

(The whole test is those two asserts: the init body runs at load, and `run_init()` empty. `session_name()` is answered from the client's own argument, which is the test's given name, so no socket is involved and this test is the unit half of the E2E the fixture test is the half of.)

- [ ] **Step 2: Run the test and watch it fail**

Run: `cargo test -p neotmux-lua --test session_name`
Expected: FAIL — `build_named` does not exist.

- [ ] **Step 3: Implement**

In `crates/neotmux-lua/src/api/mod.rs`, in `Shared`:

```rust
    /// Which session this client is in, as the host knows it. Beside `home` in kind:
    /// a fact about the process rather than about a pane, and nothing in the Lua layer
    /// could set it.
    pub(crate) session_name: String,
```

In the `neotmux` table, next to where `home` and `width` are set:

```rust
    {
        let name = session_name.clone();
        let session_name = lua.create_function(move |_lua, (): ()| {
            Ok::<String, mlua::Error>(name.clone())
        })?;
        neotmux.set("session_name", session_name)?;
    }
```

In `crates/neotmux-lua/src/lib.rs`:

```rust
    pub fn build(layers: Layers, panes: Rc<dyn PaneRead>) -> anyhow::Result<Self> {
        Self::build_named("default", layers, panes)
    }

    pub fn build_named(
        session_name: impl Into<String>,
        layers: Layers,
        panes: Rc<dyn PaneRead>,
    ) -> anyhow::Result<Self> {
        // ... existing body of build ...
        // with session_name: session_name.into() added to the api::Shared{ ... } literal.
        ...
    }
```

(Move the existing `build` body into `build_named` verbatim and add the one field; do not duplicate it.)

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua`
Expected: PASS, including all pre-existing `neotmux-lua` tests, which use the untouched `build`.

- [ ] **Step 5: The E2E fixture test**

Create `crates/neotmux/tests/fixtures/lua/plugin/session_name.lua`:

```lua
-- The name the client was started under, in the log: the E2E proof that the fact
-- about the session reaches the plugins at all.
neotmux.log.info('session_name ' .. neotmux.session_name())
```

In `crates/neotmux/tests/fixtures.rs`, next to the other `start(...)`+`wait_for_log` tests:

```rust
#[test]
fn a_named_client_is_told_its_name_by_the_api() {
    // Two sessions in the same isolation, so the assertion is about which one, not
    // whether one is running.
    let (mut nt, _init) = start("require('plugin.session_name')", "/bin/cat");
    nt.wait_for_log("session_name default", PATIENCE);
}
```

(Start with no name: the harness launches `neotmux` with no positional, so the name is the default. The *named* assertion is Task 5's, where the harness learns a name.)

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux-lua/src/lib.rs crates/neotmux-lua/src/api/mod.rs crates/neotmux-lua/tests/session_name.rs crates/neotmux/tests/fixtures/ crates/neotmux/tests/fixtures.rs
git commit -m "the plugin's one fact about which session it is in"
```

---

### Task 5: Named sessions, end to end

**Files:**
- Modify: `crates/neotmux/tests/harness/mod.rs` (the `spawn` tail factored into `spawn_with`, a `kill` verb, a new `start_named_on`)
- Test: create `crates/neotmux/tests/sessions.rs`

**Interfaces:**
- Consumes: Tasks 1–4, all landed.
- Produces: `Neotmux::start_named_on(runtime: &Path, session: &str, layer: &tempfile::TempDir, shell: &str)` and `Neotmux::kill(&mut self)` — the two the two-clients plan reuses, by name (its harness step points at them).

- [ ] **Step 1: The harness gains a shared-runtime client and a kill**

In `crates/neotmux/tests/harness/mod.rs`:

1. Split `spawn`. Move everything from the `openpty` down through the `Neotmux { .. }` construction (reader thread, parser, `wait_until_raw`, struct init) into a new
   `fn spawn_with(cmd: CommandBuilder, isolation: &Path, rows: u16, cols: u16) -> Self`.
   The existing `spawn` keeps making its own tempdir and `base_command`, then calls
   `spawn_with`. The `_isolation` field becomes `std::path::PathBuf`; the tempdir it used
   to own is now held by the caller that made it — the existing in-file callers are
   `spawn` itself and nothing else (its `start_layered` chain all goes through `spawn`),
   so the ownership change stays in this file. `end_session` keeps pointing at
   `isolation` in `spawn_with`, exactly as it does in `spawn` today.

2. Add, beside `wait_for_log`:

```rust
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
        cmd.env("NEOTMUX_RUNTIME", layer.path().to_string_lossy());
        cmd.arg(session);
        cmd.env("SHELL", resolve_shell(shell));
        Self::spawn_with(cmd, runtime, ROWS, COLS)
    }
```

(`exited_cleanly` is already in this file at line 426; read it first — it waits and reports, and `kill` above is `socket.rs`'s `child.kill()` plus that wait.)

- [ ] **Step 2: The tests, in their own file**

Create `crates/neotmux/tests/sessions.rs`:

```rust
//! Named sessions over the binary: `work` starts beside `default`, `ls` says which of
//! them is running, `kill work` ends only `work`, and a name that would not be a file in
//! the directory is refused before it is a file.

use std::process::Command;
use std::time::Duration;

mod harness;
mod session;

use harness::Neotmux;

/// The test's own `$XDG_RUNTIME_DIR`. `_end` is declared first so it drops first — ended
/// whatever session a test left behind while the directory its lock is in still exists,
/// the order `socket.rs`'s TestRuntime already keeps.
struct Runtime {
    _end: session::EndsSession,
    dir: tempfile::TempDir,
}

impl Runtime {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("runtime dir");
        let _end = session::EndsSession::new(dir.path());
        Self { _end, dir }
    }
}

fn bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_neotmux"))
}

/// `ls` is a connect-probe of whichever directory it is told about, so it runs under the
/// test's, and it is read as words, one session per line, name first.
fn ls(rt: &Runtime) -> (i32, String, String) {
    let out = Command::new(bin())
        .env("XDG_RUNTIME_DIR", rt.dir.path())
        .env("HOME", rt.dir.path())
        .arg("ls")
        .output()
        .expect("run neotmux ls");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn kill(rt: &Runtime, name: Option<&str>) -> (i32, String) {
    let mut cmd = Command::new(bin());
    cmd.env("XDG_RUNTIME_DIR", rt.dir.path())
        .env("HOME", rt.dir.path());
    cmd.arg("kill");
    if let Some(name) = name {
        cmd.arg(name);
    }
    let out = cmd.output().expect("run neotmux kill");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn a_named_session_starts_and_ls_shows_only_what_is_running() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);
    let _nt = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");

    let (code, out, err) = ls(&rt);
    assert_eq!(code, 0, "one session running, and ls says so: {out:?} {err:?}");
    assert!(
        out.lines().any(|l| l.starts_with("work") && l.contains("pid ")),
        "work is running, with its server's pid on the line the lock file gave it: {out:?}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("default")),
        "default was never started in this test's dir: {out:?}"
    );
}

#[test]
fn a_second_client_of_the_same_name_gets_the_first_client_s_pane() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);

    let mut first = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    first.settle(Duration::from_millis(300));
    first.send("hello-two\r");
    first.wait_for_screen("hello-two", Duration::from_secs(10));
    first.kill(); // the client goes; the pane stays, the way a detach already proves, with a name on it

    let mut second = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    second.wait_for_screen("hello-two", Duration::from_secs(10));
    // And the second one is not handed a fresh session: a fresh one would be an empty
    // shell that has never seen `hello-two`, which is to say this wait would never end.
}

#[test]
fn kill_work_leaves_default_running_and_vica_versa() {
    let rt = Runtime::new();
    let layer = harness::runtime(&[("init.lua", "")]);
    let a = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    let b = Neotmux::start_named_on(rt.dir.path(), "default", &layer, "/bin/cat");
    let (_a, _b) = (a, b); // both held until the end, for the drop order at the bottom

    let (code, out) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "kill work: {out:?}");

    let (code, out, err) = ls(&rt);
    assert_eq!(code, 0, "default is still running: {out:?} {err:?}");
    assert!(
        out.lines().any(|l| l.starts_with("default")),
        "default survived kill work: {out:?}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("work")),
        "work is gone: {out:?}"
    );
}

#[test]
fn a_name_that_would_not_be_a_file_is_refused_and_leaves_nothing() {
    let rt = Runtime::new();
    // `a:b` is the spec's; `ls work` is the one the Review Focus names. All four are
    // refusals, and they are said, not warned.
    for args in [
        vec!["kill", "a:b"],
        vec!["kill", "-lead"],
        vec!["a:b"],
        vec!["ls", "work"],
    ] {
        let out = Command::new(bin())
            .env("XDG_RUNTIME_DIR", rt.dir.path())
            .env("HOME", rt.dir.path())
            .args(args.iter().map(|s| s.as_str()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("run");
        let code = out.status.code().unwrap_or(-1);
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_ne!(
            code, 0,
            "a refused invocation was accepted: {args:?} — {said:?}"
        );
    }
    // Neither one is a file in the directory: the refusal was at the argument, which is
    // what keeps this from being a leak.
    let leaked: Vec<String> = std::fs::read_dir(rt.dir.path().join("neotmux"))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains(':') || n.starts_with('-'))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leaked.is_empty(),
        "a refused name left a file in {}'s dir: {leaked:?}",
        rt.dir.path().display()
    );
}

#[test]
fn kill_says_which_name_it_is_refusing_to_guess_at() {
    let rt = Runtime::new();

    let (no_code, said) = kill(&rt, Some("nowhere"));
    assert_ne!(no_code, 0, "nothing running, and kill says it is not running: {said:?}");
    assert!(said.contains("not running"), "the refusal names the case: {said:?}");

    let layer = harness::runtime(&[("init.lua", "")]);
    let _nt = Neotmux::start_named_on(rt.dir.path(), "work", &layer, "/bin/cat");
    let (code, said) = kill(&rt, Some("work"));
    assert_eq!(code, 0, "kill work: {said:?}");
    let (code, out, err) = ls(&rt);
    assert_eq!(code, 1, "work is gone, and ls with nothing running exits 1: {out:?} {err:?}");
}

```

- [ ] **Step 3: Run the file and its neighbours**

Run: `cargo test -p neotmux --test sessions`
Expected: PASS. The one that fails while the name has not reached the server is
`a_second_client_of_the_same_name_gets_the_first_client_s_pane` — the regression the
feature exists to pass, and the one to look at first if this task is the one in the
red.

Run: `cargo test -p neotmux --test socket`
Expected: PASS, unchanged. `socket.rs` starts no names, so it is the `default`
regression in this task, and it was the one the earlier tasks already kept green at their
gates.

- [ ] **Step 4: Commit**

```bash
git add crates/neotmux/tests/
git commit -m "named sessions, end to end: work starts, ls says so, kill work leaves default"
```

### Task 6: What the person reads

**Files:**
- Modify: `README.md` (the "Detach and attach are in." paragraph and the "Also out of v0.1:" line)
- Modify: `docs/plugins.md` (next to `neotmux.detach()`/`neotmux.quit()` at around line 635)
- Modify: `CHANGELOG.md` (Unreleased)

**Interfaces:** Consumes everything above; produces the words.

- [ ] **Step 1: README**

In the "Detach and attach are in." paragraph, after "A re-attach is a plugin restart…" change the closing sentence (currently "Multiple sessions, naming one, listing them, and two clients on one session at once are still out.") to:

```
More than one session is in: `neotmux work` starts it and a second `neotmux work`
attaches to it, `ls` lists the running ones and `kill [name]` ends one of them. The
name is a filename — letters, digits, `-` and `_`, none first — and so is `neotmux --kill`,
which is `kill` now. Two clients on one session at once is still out.
```

In "Also out of v0.1:" remove "multiple sessions" (two clients stays out until the other plan lands).

- [ ] **Step 2: `docs/plugins.md`**

Next to the `neotmux.quit()` / `neotmux.detach()` pair (line 635), one line in the same list and weight:

```
neotmux.session_name()    -- the name this client attached under; 'default' where it had none
```

and one sentence after the list: "A plugin that wants to behave differently by session branches on it; there is no per-session `init.lua`."

- [ ] **Step 3: `CHANGELOG.md`**

Under `## Unreleased`, in the file's existing register:

```
### Sessions by name, and `kill` where `--kill` was

`neotmux work` starts a session called `work` where nothing is running and attaches to
it where one is; `ls` shows the running ones, one per line with the server's pid, and
`kill [name]` ends one of them, `default` where no name is given — which is what a bare
`kill` does, and what `--kill` used to be, and is no more. A name is letters, digits,
`-` and `_` (none first), one to sixty-four characters, because it becomes a filename,
and a name that would not is refused before anything is created. `C-b d` detaches to the
same server it always did; running `neotmux work` again finds the session, as running
`neotmux` again already found `default`. Nothing about the wire changed: the name is a
path, and the socket was one file per session the day it was written.
```

- [ ] **Step 4: Whole-suite run**

Run: `make check`
Expected: PASS (fmt, clippy `-D warnings`, all tests).

- [ ] **Step 5: Commit**

```bash
git add README.md docs/plugins.md CHANGELOG.md
git commit -m "the words: names are in, --kill is not"
```

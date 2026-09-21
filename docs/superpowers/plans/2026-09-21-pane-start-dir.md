# Pane Start Directory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new pane's shell starts in the directory the user expects — where neotmux was started, or where the focused pane itself is — instead of always in `$HOME`.

**Architecture:** The server process that spawns shells inherits its working directory from the client that started it, so `std::env::current_dir()` in the server *is* the startup directory and needs no plumbing. For a split or a new window, the focused pane's live directory is read the same way `paneinfo` already does: foreground pid via `Shells::foreground`, then `/proc/<pid>/cwd` in `procinfo`. One helper in `Session` answers "where does a new pane begin", and each of the three spawn sites (first pane, `pane.split`, `window.create`) sets that directory on the `CommandBuilder` before handing it to `PtyHandle::spawn`.

**Tech Stack:** Rust workspace (`neotmux`, `neotmux-term`, `portable-pty` 0.9), PTY-driven integration tests in `crates/neotmux/tests/` with the `Neotmux` harness and the checked-in fixture plugins (`paneinfo`, `splitter`).

**Spec:** GitHub issue #4 — "Starting neotmux drops the shell in `$HOME` instead of the directory it was started from". Decisions confirmed with the author on 2026-09-21:

1. **Split:** the focused pane's current directory (its foreground process, via `foreground()`), falling back to the startup directory when that directory is gone or unreadable.
2. **New window:** the *same rule as a split* (tmux `new-window` parity), not the startup directory.
3. **Login shell (`-` argv0 prefix from `new_default_prog`):** **kept as-is**; not changed in this work.
4. **ABI (plugin-specified pane directory):** **not** added; left as a follow-up noted on the issue. There is no `PaneOpen` op today — `Session::new`, `apply_split` and `apply_window_create` are the only three spawn sites (checked against `SessionOp` in `crates/neotmux-proto/src/msg.rs:73-93`).

**Root cause (confirmed by reading the code, not assumed):** `PtyHandle::default_shell_command()` (`crates/neotmux-term/src/pty.rs:66`) builds `CommandBuilder::new_default_prog()` with no cwd. `portable-pty` 0.9.0 `CommandBuilder::as_command` (`registry src .../portable-pty-0.9.0/src/cmdbuilder.rs:498-526`) maps an unset cwd **explicitly** to the home directory (`get_home_dir`, line 560) rather than inheriting. All three spawn sites in `crates/neotmux/src/session.rs` (`Session::new` ~line 92, `apply_split` ~line 322, `apply_window_create` ~line 354) pass this bare command. Because portable-pty *requires* a directory, the fix must always name one — an unset cwd is not "inherit", it is "use `$HOME`".

## Global Constraints

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` stay clean (`Makefile: check`).
- `cargo test --workspace` stays green; every PTY-driven test must end the session it starts (existing harness `session::EndsSession`).
- Linux-only `/proc` reads stay behind the existing `procinfo` module; a missing answer is `None`, never an error (contract of `procinfo::read`).
- Repo comment style is explanatory prose; new comments match the existing voice of `session.rs`/`procinfo.rs`.
- Commit messages: short, imperative, prefixed by the area (`session:`, `tests:`, `harness:`), matching `git log`.

## Review Focus

Failure classes the spec implies but no test will exercise, in order of bite-likelihood, and where each is pinned:

1. **Focused pane's directory was deleted after the `cd`** — `/proc/<pid>/cwd` read fails; the new pane must fall back to the startup directory, not `$HOME` and not an error. → Pinned by Task 1 test 3.
2. **`split` moves focus to the new pane** — if the "focused pane" is read *after* `self.windows...split(...)`, "the focused pane" is the pane being made, which has no shell and no directory; it would silently fall back and mask the fix. → Pinned by the Task 2 implementation ordering (dir read before the tree mutation; test 2 is the canary).
3. **`window.create` while a hidden window is showing** — `start_dir` answers from the *shown* window's focused pane (the pane the user is looking at is the "focused pane"); a pane in a hidden window cannot be the one a person means. → Pinned by the Task 2 comment at that call site; no dedicated test (hidden-window focus state is not expressible through the fixtures' keybinding).
4. **Server cwd diverges from the client's** — the client spawns the server with `std::process::Command` without a `current_dir`, so the server inherits it (`crates/neotmux/src/start.rs` `spawn()`); a future change adding `current_dir` to that spawn silently breaks the rule. → Pinned by the comment at `Session::new`; test 1 is the canary (it goes through client → server).
5. **`current_dir()` itself fails in the server** (its cwd deleted too) — answer is `None`, the command is left unset, and portable-pty's documented fallback (`$HOME`) applies. Accepted, noted in the `start_dir` comment; untestable without deleting the server's own cwd.

---

### Task 1: Failing tests for the start-directory rule

**Files:**
- Modify: `crates/neotmux/tests/harness/mod.rs` (add a `cwd` parameter to `spawn`; add `start_in_cwd` / `start_layered_in` / `start_sized_in`)
- Modify: `crates/neotmux/tests/fixtures.rs` (add `start_in` helper + 3 tests, near the existing `a_panes_directory_follows_a_cd_in_it` at ~line 1535)

**Interfaces:**
- Consumes: existing `harness::runtime`, `harness::fixture_layer`, `Neotmux::send / wait_for_log / wait_for_status`, fixture plugins `plugin.paneinfo` (logs `paneinfo <id> command=<comm> cwd=<dir>`) and `plugin.splitter` (C-b = `\x02` splits vertically; new pane is id 2).
- Produces: `Neotmux::start_in_cwd(dir: &tempfile::TempDir, shell: &str, cwd: &Path) -> (Neotmux, tempfile::TempDir)` pair-shaped via `start_in` in fixtures.rs — the vehicle Tasks 2's verification uses.

The paneinfo log line shape (from `tests/fixtures/lua/plugin/paneinfo.lua` `line()`):
`paneinfo 2 command=bash cwd=/path` — `cwd=None` when unknown. The existing tests at `fixtures.rs:1509-1545` show the exact assertion idiom (`wait_for_log` + `line_with`).

- [ ] **Step 1: Extend the harness to start a client in a chosen directory**

In `crates/neotmux/tests/harness/mod.rs`:

Change `spawn`'s signature and body — add the trailing parameter and the two lines before `let child = ...`:

```rust
    fn spawn(
        exe: &Path,
        runtime: Option<String>,
        shell: &str,
        rows: u16,
        cols: u16,
        cwd: Option<&Path>,
    ) -> Self {
```

```rust
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
```

Update the two existing callers to pass `None` (`start_sized` at ~line 91 and `start_installed` at ~line 104 — each ends in `Self::spawn(...)`); `cargo check -p neotmux --tests` will name any other I missed and add the chain that mirrors the existing one:

```rust
    /// Started from a directory of the caller's choosing — the point of the tests that
    /// ask where the first pane begins. `dir` is the runtime layer, as before.
    pub fn start_in_cwd(dir: &tempfile::TempDir, shell: &str, cwd: &Path) -> Self {
        Self::start_layered_in(&[dir.path()], shell, cwd)
    }

    pub fn start_layered_in(layers: &[&Path], shell: &str, cwd: &Path) -> Self {
        Self::start_sized_in(layers, shell, ROWS, COLS, cwd)
    }

    pub fn start_sized_in(
        layers: &[&Path],
        shell: &str,
        rows: u16,
        cols: u16,
        cwd: &Path,
    ) -> Self {
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
```

- [ ] **Step 2: Add the `start_in` fixture helper and the three tests**

In `crates/neotmux/tests/fixtures.rs`, next to `start` (line 15):

```rust
/// As `start`, but neotmux is launched from `cwd`: the tests that ask where a pane *begins*
/// need a beginning they chose.
fn start_in(init: &str, shell: &str, cwd: &std::path::Path) -> (Neotmux, tempfile::TempDir) {
    let dir = harness::runtime(&[("init.lua", init)]);
    let fixtures = harness::fixture_layer();
    let nt = Neotmux::start_layered_in(&[dir.path(), &fixtures], shell, cwd);
    (nt, dir)
}
```

Note: `tempfile::tempdir()`'s path is not canonicalised, while `/proc/<pid>/cwd` reports the canonical one — if the system's temp dir sits behind a symlink the assertion string would disagree with itself. Every test below therefore asserts on the canonicalised path while still *launching* from the raw one (`std::env::current_dir()` in the server holds whatever was launched). On a machine whose temp dir is a plain directory the two are identical and the test is unaffected.

After `a_panes_directory_follows_a_cd_in_it` (line ~1545), add:

```rust
/// Where the first pane begins: the directory neotmux was started from, not the user's home
/// — the one a person means when they drop a multiplexer into a checkout. The home
/// directory is the isolation tempdir the harness hands the session, and it is *different*
/// from `launch`, which is what makes the timeout below a failure rather than a flake.
#[test]
fn the_first_pane_begins_where_neotmux_was_started() {
    let launch = tempfile::tempdir().expect("launch dir");
    let home = std::fs::canonicalize(launch.path()).expect("canonical launch");
    let (mut nt, _init) = start_in("require('plugin.paneinfo')", "bash", launch.path());

    let log = nt
        .wait_for_log("paneinfo 1 command=bash", PATIENCE);
    assert!(
        line_with(&log, "paneinfo 1 command=bash").contains(&format!("cwd={}", home.display())),
        "{log}"
    );
}

/// Where a split begins: where the focused pane itself is, the way the
/// `#{pane_current_path}` idiom means it — the pane's own directory, read from the process
/// running in it, not the directory neotmux came from. The new pane is id 2 and the
/// assertion is on *its* line: pane 1 already reports the target directory from the `cd`,
/// so the pane number is what carries the claim.
#[test]
fn the_split_begins_in_the_focused_panes_directory() {
    let launch = tempfile::tempdir().expect("launch dir");
    let home = std::fs::canonicalize(launch.path()).expect("canonical launch");
    let target = home.join("elsewhere");
    std::fs::create_dir(&target).expect("subdir");

    let (mut nt, _init) = start_in(
        "require('plugin.paneinfo') require('plugin.splitter')",
        "bash",
        launch.path(),
    );
    nt.wait_for_log(&format!("paneinfo 1 command=bash cwd={}", home.display()), PATIENCE);

    nt.send(&format!("cd {}\n", target.display()));
    nt.wait_for_log(&format!("cwd={}", target.display()), PATIENCE);

    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    let log = nt.wait_for_log("paneinfo 2 ", PATIENCE);
    assert!(
        line_with(&log, "paneinfo 2 ").contains(&format!("cwd={}", target.display())),
        "{log}"
    );
}

/// The split's fallback: the focused pane's directory is gone — deleted out from under it —
/// so the only directory left to answer with is where neotmux was started. Not `$HOME`,
/// which is what the un-pinned path silently chooses, and not an error, which would cost
/// the split.
#[test]
fn the_split_whose_focused_pane_left_a_deleted_directory_begins_where_neotmux_was_started() {
    let launch = tempfile::tempdir().expect("launch dir");
    let home = std::fs::canonicalize(launch.path()).expect("canonical launch");
    let vanished = home.join("vanished");
    std::fs::create_dir(&vanished).expect("subdir");

    let (mut nt, _init) = start_in(
        "require('plugin.paneinfo') require('plugin.splitter')",
        "bash",
        launch.path(),
    );
    nt.wait_for_log(&format!("paneinfo 1 command=bash cwd={}", home.display()), PATIENCE);

    nt.send(&format!("cd {}\n", vanished.display()));
    nt.wait_for_log(&format!("cwd={}", vanished.display()), PATIENCE);
    std::fs::remove_dir_all(&vanished).expect("delete out from under the pane");

    nt.send("\x02"); // C-b: split
    nt.wait_for_log("splitter pane_opened 2", PATIENCE);

    let log = nt.wait_for_log("paneinfo 2 ", PATIENCE);
    assert!(
        line_with(&log, "paneinfo 2 ").contains(&format!("cwd={}", home.display())),
        "{log}"
    );
}
```

- [ ] **Step 3: Run the new tests and verify they fail for the right reason**

Run: `cargo test -p neotmux --test fixtures begins`
Expected: all three FAIL. `the_first_pane_begins_where_neotmux_was_started` asserts `cwd=<launch>` on the `paneinfo 1` line — before the fix that line carries the isolation home dir. Check the assertion output contains `cwd=<isolation>` (a different tempdir), not a panic or a harness error: if it panics, the harness change is broken, fix that first — a failing test that fails for the wrong reason is a broken test.

- [ ] **Step 4: Commit the failing tests**

```bash
git add crates/neotmux/tests/harness/mod.rs crates/neotmux/tests/fixtures.rs
git commit -m "tests: a pane should begin where it is asked to begin"
```

---

### Task 2: Answer "where does a new pane begin" at the three spawn sites

**Files:**
- Modify: `crates/neotmux/src/procinfo.rs` (make `cwd` `pub(crate)`)
- Modify: `crates/neotmux/src/session.rs` (helper + 3 call sites)

**Interfaces:**
- Consumes: `Shells::foreground(&self, pane: PaneId) -> Option<i32>` (`crates/neotmux/src/shells.rs:182`), `PtyHandle::default_shell_command() -> CommandBuilder` (`crates/neotmux-term/src/pty.rs:66`), `PaneLayout::focus() -> Option<PaneId>` (`crates/neotmux/src/panes.rs:141`), `portable_pty::CommandBuilder::cwd<P: AsRef<Path>>`.
- Produces: `Session::start_dir(&self, pane: Option<PaneId>) -> Option<PathBuf>` — the single answer for every future spawn site.

- [ ] **Step 1: Expose the cwd read**

In `crates/neotmux/src/procinfo.rs`, change:

```rust
fn cwd(pid: i32) -> Option<String> {
```

to:

```rust
/// The process's working directory: `/proc/<pid>/cwd`, `None` when the process is gone.
/// A deleted directory is the same answer — the link cannot be read — which is what a
/// caller asking "where is it, live, right now" wants to hear.
pub(crate) fn cwd(pid: i32) -> Option<String> {
```

- [ ] **Step 2: Add the helper to `Session`**

In `crates/neotmux/src/session.rs`, add to `impl Session` (near `refresh_pane_info`), plus the `PathBuf` import at the top (add `use std::path::PathBuf;` in the imports block):

```rust
    /// Where a new pane's shell begins, and why the answer must always be one. Portable-pty
    /// does not inherit a working directory: an unset one becomes the user's home, so a
    /// caller that leaves it unset is not saying "as you like", it is saying `$HOME`, and
    /// that is the bug this exists to stop. The focused pane's own directory leads — the
    /// foreground process, which is the shell while it is idle and the command it is
    /// running otherwise — and the server's, which is where the client that started it
    /// started it, is the answer when the pane's directory is gone or the pane does not
    /// exist (the first pane).
    fn start_dir(&self, pane: Option<PaneId>) -> Option<PathBuf> {
        if let Some(dir) = pane
            .and_then(|pane| self.shells.foreground(pane))
            .and_then(procinfo::cwd)
            .filter(|dir| std::path::Path::new(dir).is_dir())
        {
            return Some(dir.into());
        }
        std::env::current_dir().ok()
    }
```

- [ ] **Step 3: First pane — `Session::new`**

Replace the spawn in `Session::new` (~line 92):

```rust
        shells.open(
            first,
            PtyHandle::default_shell_command(),
            pane_size(windows.rect(first), first)?,
        )?;
```

with:

```rust
        // The session's own directory: the server is a grandchild of the directory neotmux
        // was started from, and `start_dir` cannot be asked for it yet — there is no pane
        // to point at. The pane that is about to be made is the session's first.
        let mut cmd = PtyHandle::default_shell_command();
        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }
        shells.open(first, cmd, pane_size(windows.rect(first), first)?)?;
```

- [ ] **Step 4: Split — `apply_split`**

In `apply_split`, read the directory **before** the tree is split (the split may move focus to the pane it is making, and "the focused pane" after the split would have no shell to read), and carry it into the spawn:

Before:

```rust
        let started = pane_size(self.windows.rect(new), new).and_then(|size| {
            self.shells
                .open(new, PtyHandle::default_shell_command(), size)
        });
```

After:

```rust
        // Read now, before `split`: focus may land on `new` the moment the tree allows,
        // and a pane with no shell to read would be answered for by the fallback, hiding
        // the rule it is trying to say.
        let start_dir = self.start_dir(self.windows.current().focus());
        let started = pane_size(self.windows.rect(new), new).and_then(move |size| {
            let mut cmd = PtyHandle::default_shell_command();
            if let Some(dir) = start_dir {
                cmd.cwd(dir);
            }
            self.shells.open(new, cmd, size)
        });
```

(`move` is required: the closure owns the `Option<PathBuf>`; the `&mut self` borrow it also carries is what `and_then` already needed.)

- [ ] **Step 5: New window — `apply_window_create`**

Same shape:

Before:

```rust
        let first = self.windows.create(window, self.surface);
        let started = pane_size(self.windows.rect(first), first).and_then(|size| {
            self.shells
                .open(first, PtyHandle::default_shell_command(), size)
        });
```

After:

```rust
        let first = self.windows.create(window, self.surface);
        // The shown window's focused pane: the one a person is looking at when they ask
        // for a window is the one they mean to carry forward, whether the window the
        // question came from is the one on display or not.
        let start_dir = self.start_dir(self.windows.current().focus());
        let started = pane_size(self.windows.rect(first), first).and_then(move |size| {
            let mut cmd = PtyHandle::default_shell_command();
            if let Some(dir) = start_dir {
                cmd.cwd(dir);
            }
            self.shells.open(first, cmd, size)
        });
```

- [ ] **Step 6: Watchdog for the server's directory — comment, not code**

`spawn()` in `crates/neotmux/src/start.rs` (the client starting `--server`) must keep inheriting the client's directory — it must not gain a `current_dir`. Add where `command` is built:

```rust
    // No `current_dir` on the server: its own directory is the startup directory, and
    // every pane that is not told where to go from asks one of its children.
```

- [ ] **Step 7: Run the three tests — they pass**

Run: `cargo test -p neotmux --test fixtures begins`
Expected: 3 passed, 0 failed. If `the_split_begins_in_the_focused_panes_directory` still fails, do not add another fix — re-read the assertion output and check whether the `paneinfo 2` line shows the *isolation home* (fallback fired: focus already moved, or `foreground()` answered `None`) or `cwd=None` (the `/proc` read failed).

- [ ] **Step 8: Whole-suite verification**

Run: `cargo check --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean. The paneinfo/fixture suite is the canary for anything session.rs touched.

- [ ] **Step 9: Commit**

```bash
git add crates/neotmux/src/procinfo.rs crates/neotmux/src/session.rs crates/neotmux/src/start.rs
git commit -m "session: a pane begins where the one before it is"
```

---

### Task 3: Record the decisions

**Files:**
- Modify: `CHANGELOG.md` (under `## Unreleased`, a new `### ...` entry after the existing one, matching its prose style)

- [ ] **Step 1: Changelog entry**

```markdown
### A pane begins where it is asked to begin

A new pane's shell used to start in the user's home directory, wherever neotmux was run
from and whatever the pane it split from was in: the working directory was never named,
and the library neotmux forks through turns an unset one into `$HOME` rather than leaving
it inherited. Now the first pane begins in the directory neotmux was started from — the
multiplexer behaves like every other terminal a hand can drop it into — and a split, and
a new window, begin in the focused pane's own directory, the read the status bar already
uses for `#{pane_current_path}`-style answers, from the process the pane is running.
When that directory has been deleted out from under the pane, the answer is the startup
directory rather than the home one, and the split still happens.

The decision, in one place: the shell neotmux starts is still a login shell (the `-`
argv0 the spawner's default uses), and there is still no way for a plugin opening a pane
to choose the directory — both known, and neither changed by this.
```

- [ ] **Step 2: Say the decisions on issue #4**

```bash
gh issue comment 4 --body "$(cat <<'EOF'
Decided, implemented:

- **New window:** same rule as a split — the *shown* window's focused pane's directory, startup directory as fallback. Matches tmux `new-window` and keeps one rule to remember.
- **Login shell:** kept. `-` argv0 is tmux's default too; changing it would rewrite every prompt config out there.
- **ABI:** deliberately not added. There is no pane-open op for plugins to attach it to, and inventing one for a directory argument is the wrong first step. Revisit if a pane-open op arrives.

Tests pin all three of the first pane, the split, and the deleted-directory fallback, the way the status bar's `pane.info` reads them (foreground pid, `/proc/<pid>/cwd`).
EOF
)"
```

- [ ] **Step 3: Commit and close**

```bash
git add CHANGELOG.md
git commit -m "docs: the rules a new pane's directory follows"
```

Then ask the author to close issue #4 (or `gh issue close 4 --comment` if they prefer the close from this side — do not self-close without the ask).

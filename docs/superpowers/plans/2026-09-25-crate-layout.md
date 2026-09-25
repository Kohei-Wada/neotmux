# Crates you can read the shape of — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The workspace's crates sit at the repository root under role names, `neotmux` is split into client / server / sys / binary, and every `lib.rs` / `mod.rs` holds exports only with directories as modules — with no behaviour change.

**Architecture:** A pure move. Each task relocates files with `git mv`, rewrites paths, and is proven by `make check` passing with the same number of tests as the baseline. The only code that changes is `use` paths, `mod` lines, visibility needed to cross a new crate boundary, and three file renames.

**Tech Stack:** Cargo workspace (Rust 2024), `git mv`, `sed`, `cargo check` / `make check`, `cargo tree`.

**Spec:** `docs/superpowers/specs/2026-09-25-crate-layout-design.md`

## Global Constraints

- No behaviour change. No function body changes except `use` paths, and the visibility a new crate boundary needs (`pub(crate)` → `pub`).
- Every move is `git mv` (history follows the file).
- No compatibility re-exports under old names; every use of a moved path is rewritten.
- Specs and plans already in `docs/superpowers/` are not edited (they are the record) — except this plan's own spec.
- `neotmux-layout` never depends on `neotmux-lua`.
- `neotmux-client` depends on `neotmux-proto` and `neotmux-sys` only.
- Every task ends with `make check` green and the test count equal to the baseline taken in Task 1 Step 1.
- Work only in `.worktrees/crate-layout` (branch `crate-layout`).
- Commit messages: a lowercase sentence saying what is now true, then
  ```
  Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
  ```

### How to count tests (used by every task)

```bash
make check > "$W/check.log" 2>&1; echo exit=$?
grep "test result" "$W/check.log" | awk '{p+=$4; f+=$6; i+=$8} END {print "passed="p" failed="f" ignored="i}'
```

`$W` is the plan's workspace directory (printed by `sdd-workspace`). The baseline is recorded in the ledger in Task 1 Step 1; every later task must print the same `passed` and `ignored`, and `failed=0`. A known flaky test (`sock::tests::a_bind_replaces_the_socket_a_dead_server_left_and_makes_it_private`, and #44's socket test) that fails once is re-run once; a second failure stops the task.

## Review Focus

1. A test that moves crates but is silently no longer compiled (a `tests/` file left behind a moved `Cargo.toml`, or a `mod` line dropped) — the count check catches it only if the baseline is exact; compare per-crate test counts too, not just the total.
2. Visibility widened past what a boundary needs — an item made `pub` that nothing outside its new crate uses. Each `pub(crate)` → `pub` must have a caller in another crate.
3. Runtime paths resolved from `CARGO_MANIFEST_DIR` (`../../runtime` → `../runtime`) and `tests/fixtures` — a wrong one fails only the tests that load the shipped layer, which is most of `neotmux-lua`'s.
4. `cargo install --path` in the `Makefile` and anything else that names a crate directory (CI, NOTICE, README) — `make install` is not run by `make check`.
5. A glob re-export that makes a previously private item public (`mod x; pub use x::*;` where `x` was private) — the spec requires `pub(crate) use` for those.

---

## Path mapping (every task that moves a module rewrites by these tables)

**Task 2 — crate renames**

| old | new |
|---|---|
| `neotmux_core` | `neotmux_layout` |
| `neotmux_term` | `neotmux_pty` |
| `neotmux-core` (Cargo) | `neotmux-layout` |
| `neotmux-term` (Cargo) | `neotmux-pty` |

**Task 4 — the split** (inside the old `neotmux` crate, `crate::` paths)

| old (in `neotmux`) | new |
|---|---|
| `crate::sock`, `neotmux::sock` | `neotmux_sys::sock` |
| `crate::link` | `neotmux_sys::link` |
| `crate::signals` | `neotmux_sys::signals` |
| `crate::tty` | `neotmux_sys::tty` |
| `crate::logging`, `neotmux::logging` | `neotmux_sys::logging` |
| `crate::procinfo` | `neotmux_sys::procinfo` |
| `crate::client::X` (inside client) | `crate::X` |
| `crate::server::X` (inside server) | `crate::X` |
| `neotmux::client::X` (main.rs) | `neotmux_client::X` |
| `neotmux::server::X` (main.rs, tests) | `neotmux_server::X` |

**Task 5 — directories** (a module moved into a directory is flattened into it)

| crate | old module | new module |
|---|---|---|
| proto | `draw`, `frame`, `read`, `width` | `screen` |
| proto | `geom` (file renamed `size.rs`), `geometry`, `panerects`, `reserve`, `grid` | `geom` |
| proto | `msg`, `wire` (file renamed `codec.rs`) | `wire` |
| proto | `ids`, `keys`, `event`, `level`, `paneinfo` | unchanged |
| pty | `pty`, `wakeup` | `child` |
| pty | `state`, `scrollback`, `evictions`, `lines`, `wrap` | `screen` |
| pty | `query`, `hvp` | `parse` |
| pty | `terminals` | unchanged |
| lua | `ambient`, `ops`, `paneinfo`, `window`, `clipboard`, `runner` | `host` |
| lua | `jobs`, `timers`, `budget`, `overrun` | `sched` |
| lua | `draw` (file renamed `buffers.rs`), `float`, `reserve` | `draw` |
| lua | `registry`, `source`, `commands`, `runtime`, `vm` | `plugin` |
| lua | `api` | unchanged |

So `neotmux_proto::draw::DrawCmd` → `neotmux_proto::screen::DrawCmd`, `crate::budget::Budget` → `crate::sched::Budget`, `neotmux_pty::terminals::PaneTerminals` unchanged. The root re-exports already in `neotmux-proto/src/lib.rs` (`pub use draw::{DrawCmd, …}`) keep their names and only change the module they name.

Root-level files that stay at the root keep `pub mod x;` / `mod x;` exactly as today — their paths do not move.

---

### Task 1: the crates sit at the repository root

**Files:**
- Move: `crates/neotmux*` → `./neotmux*`
- Modify: `Cargo.toml`, `Makefile`, every `CARGO_MANIFEST_DIR`-relative path in `*/tests/**/*.rs`

**Interfaces:**
- Produces: crate directories at the root; `members = ["neotmux", "neotmux-*"]`.

- [ ] **Step 1: Record the baseline**

Run the count (see "How to count tests") on the untouched tree. Expected: `exit=0`, `failed=0`. Write to the ledger: `Baseline: passed=<n> ignored=<m>`, and per crate:

```bash
grep -E "Running|test result" "$W/check.log" > "$W/baseline-per-target.txt"
```

- [ ] **Step 2: Move the crates**

```bash
for c in crates/*; do git mv "$c" "$(basename "$c")"; done
rmdir crates
```

- [ ] **Step 3: Point the workspace at them**

In `Cargo.toml`: `members = ["crates/neotmux-proto", …]` becomes

```toml
members = ["neotmux", "neotmux-*"]
```

(`neotmux-*` alone does not match `neotmux`.) And every `path = "crates/neotmux-…"` in `[workspace.dependencies]` loses its `crates/` prefix:

```bash
sed -i 's|path = "crates/|path = "|' Cargo.toml
```

In `Makefile`: `cargo install --path crates/neotmux` → `cargo install --path neotmux`.

- [ ] **Step 4: Runtime paths in the tests**

Every crate is one level shallower, so `../../runtime` is `../runtime` and `../..` is `..`:

```bash
grep -rl 'CARGO_MANIFEST_DIR")).join("../..' neotmux*/tests | xargs sed -i 's|join("../../runtime")|join("../runtime")|; s|join("../..")|join("..")|'
grep -rn 'CARGO_MANIFEST_DIR' neotmux*/tests neotmux*/src
```

Expected: no remaining `../../`; `neotmux/tests/harness/mod.rs`'s `join("tests/fixtures")` is unchanged (it is relative to its own crate).

- [ ] **Step 5: Other references to the directory**

```bash
grep -rn 'crates/' --include=*.toml --include=*.yml --include=Makefile --include=NOTICE --include=README.md --include=*.md . | grep -v '^./docs/superpowers/' | grep -v '^./target'
```

Expected: only `CHANGELOG.md` history lines, if any (leave them). Anything else (CI, NOTICE, README, `docs/plugins.md`) is rewritten without the `crates/` prefix.

- [ ] **Step 6: Check and commit**

Run the count. Expected: same `passed`/`ignored` as baseline, `failed=0`.

```bash
git add -A
git commit -F - <<'EOF'
the crates sit side by side at the root, under one glob

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 2: the crates are named for what they hold

**Files:**
- Move: `neotmux-core` → `neotmux-layout`, `neotmux-term` → `neotmux-pty`
- Modify: both crates' `Cargo.toml` `name`, root `Cargo.toml` workspace deps, every dependent `Cargo.toml`, every `.rs` naming `neotmux_core` / `neotmux_term`, `README.md`, `docs/plugins.md`, `NOTICE` where they name the crates

**Interfaces:**
- Produces: crates `neotmux-layout` (`neotmux_layout`) and `neotmux-pty` (`neotmux_pty`).

- [ ] **Step 1: Move and rename**

```bash
git mv neotmux-core neotmux-layout
git mv neotmux-term neotmux-pty
sed -i 's/^name = "neotmux-core"/name = "neotmux-layout"/' neotmux-layout/Cargo.toml
sed -i 's/^name = "neotmux-term"/name = "neotmux-pty"/' neotmux-pty/Cargo.toml
sed -i 's|^neotmux-core = { path = "neotmux-core"|neotmux-layout = { path = "neotmux-layout"|; s|^neotmux-term = { path = "neotmux-term"|neotmux-pty = { path = "neotmux-pty"|' Cargo.toml
grep -rl 'neotmux-core\|neotmux-term' neotmux*/Cargo.toml | xargs sed -i 's/neotmux-core\.workspace/neotmux-layout.workspace/; s/neotmux-term\.workspace/neotmux-pty.workspace/'
grep -rl 'neotmux_core\|neotmux_term' --include=*.rs neotmux* | xargs sed -i 's/neotmux_core/neotmux_layout/g; s/neotmux_term/neotmux_pty/g'
```

- [ ] **Step 2: What is left**

```bash
grep -rn 'neotmux-core\|neotmux-term\|neotmux_core\|neotmux_term' --exclude-dir=target --exclude-dir=superpowers . | grep -v '^./CHANGELOG.md'
```

Expected: only prose in `README.md`, `docs/plugins.md`, `NOTICE`, code comments. Rewrite each to the new name, except the design doc `docs/superpowers/specs/2026-09-19-neotmux-design.md` (excluded above: it is the record; Task 6 adds a note to it).

- [ ] **Step 3: Check and commit**

Run the count. Expected: baseline. `cargo fmt --all` first if the renamed `use` lines re-sort.

```bash
git add -A
git commit -F - <<'EOF'
the layout crate and the pty crate are called what they hold

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 3: the three small moves that make the client need nothing but proto and sys

**Files:**
- Modify: `neotmux-pty/src/state.rs` (lose `MIN_GRID`, `clamp_to_min_grid`), `neotmux-proto/src/grid.rs` (gain them), `neotmux-proto/src/lib.rs` (re-export), every user
- Move: `neotmux-pty/src/osc52.rs` → `neotmux/src/client/osc52.rs`; `neotmux-pty/src/lib.rs` loses `pub mod osc52;`; `neotmux/src/client/mod.rs` gains `mod osc52;`
- Move tests: the `#[cfg(test)]` round-trip tests at `neotmux/src/client/decode.rs` (the one using `crate::server::input::encode_key`, near line 429) and `neotmux/src/server/input.rs` (the one using `crate::client::decode::Decoder`, near line 409) → `neotmux/tests/key_round_trip.rs`

**Interfaces:**
- Produces: `neotmux_proto::grid::{MIN_GRID, clamp_to_min_grid}` (re-exported at the root as `neotmux_proto::{MIN_GRID, clamp_to_min_grid}`); `crate::client::osc52::set`; `neotmux::client::decode::Decoder` and `neotmux::server::input::encode_key` reachable from `neotmux/tests/`.

- [ ] **Step 1: Move `MIN_GRID` and `clamp_to_min_grid`**

Cut the `pub const MIN_GRID` and `pub fn clamp_to_min_grid` items (with their doc comments) out of `neotmux-pty/src/state.rs` and paste them into `neotmux-proto/src/grid.rs` unchanged. Add `MIN_GRID, clamp_to_min_grid` to the root `pub use grid::{…}` line in `neotmux-proto/src/lib.rs`. In `state.rs`, add `use neotmux_proto::{MIN_GRID, clamp_to_min_grid};` (it still calls both). Rewrite every other user:

```bash
grep -rl 'neotmux_pty::state::clamp_to_min_grid\|neotmux_pty::state::MIN_GRID\|neotmux_pty::state::{MIN_GRID' --include=*.rs . | xargs sed -i 's/neotmux_pty::state::clamp_to_min_grid/neotmux_proto::clamp_to_min_grid/g; s/neotmux_pty::state::MIN_GRID/neotmux_proto::MIN_GRID/g'
grep -rn 'MIN_GRID\|clamp_to_min_grid' --include=*.rs . | grep -v target
```

Expected: the only definitions are in `neotmux-proto/src/grid.rs`. `neotmux/src/server/session.rs`'s test line `use neotmux_pty::state::{MIN_GRID, TermState};` becomes `use neotmux_pty::state::TermState; use neotmux_proto::MIN_GRID;`.

- [ ] **Step 2: Move `osc52` to the client**

```bash
git mv neotmux-pty/src/osc52.rs neotmux/src/client/osc52.rs
sed -i '/^pub mod osc52;$/d' neotmux-pty/src/lib.rs
```

Add `mod osc52;` beside the other `mod` lines in `neotmux/src/client/mod.rs`, and change `neotmux_pty::osc52::set(text)` there to `osc52::set(text)`.

- [ ] **Step 3: Move the key round-trip tests**

Move the two `#[test]` functions named in **Files** out of their `mod tests` blocks into a new `neotmux/tests/key_round_trip.rs`, with a `//!` line saying they are here because they are the one test that needs both halves. Their bodies change only in paths: `crate::server::input::encode_key` → `neotmux::server::input::encode_key`, `crate::client::decode::Decoder` → `neotmux::client::decode::Decoder`. Make exactly what they need reachable: `pub(crate) mod decode;` → `pub mod decode;` in `client/mod.rs` (and `Decoder`/its `feed` to `pub` if they are narrower); `encode_key` to `pub` if it is narrower. Remove any `use` in the two files' `mod tests` left unused.

- [ ] **Step 4: Prove the client's pty edge is gone**

```bash
grep -rn 'neotmux_pty\|neotmux_lua\|neotmux_layout' neotmux/src/client
```

Expected: no output.

- [ ] **Step 5: Check and commit**

Run the count. Expected: baseline — the two moved tests are still counted (now under `key_round_trip`), so the total is unchanged.

```bash
git add -A
git commit -F - <<'EOF'
the minimum grid is proto's, osc 52 is the client's, and the key round trip is a test of both

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 4: the binary is split into client, server and what they share

**Files:**
- Create: `neotmux-sys/`, `neotmux-client/`, `neotmux-server/` (each `Cargo.toml` + `src/`)
- Move: `neotmux/src/{sock,link,signals,tty,logging,procinfo}.rs` → `neotmux-sys/src/`; `neotmux/src/client/*` → `neotmux-client/src/` (`mod.rs` → `lib.rs`); `neotmux/src/server/*` → `neotmux-server/src/` (`mod.rs` → `lib.rs`); `neotmux/tests/{headless,key_spec_contract,job_process}.rs` → `neotmux-server/tests/`
- Delete: `neotmux/src/lib.rs` (its module doc moves to `neotmux-sys/src/lib.rs`'s head, reworded to say what sys is; its "where a new module goes" paragraph goes to the new crates' `lib.rs` heads)
- Modify: root `Cargo.toml` workspace deps, `neotmux/Cargo.toml`, `neotmux/src/main.rs`, `neotmux/tests/*.rs`

**Interfaces:**
- Consumes: Task 3's `neotmux::client::decode::Decoder`, `neotmux::server::input::encode_key` (now `neotmux_client::decode::Decoder`, `neotmux_server::input::encode_key`).
- Produces: crates `neotmux-sys` (`neotmux_sys::{sock, link, signals, tty, logging, procinfo}`), `neotmux-client`, `neotmux-server`.

- [ ] **Step 1: The sys crate**

```bash
mkdir -p neotmux-sys/src
for m in sock link signals tty logging procinfo; do git mv neotmux/src/$m.rs neotmux-sys/src/$m.rs; done
```

`neotmux-sys/Cargo.toml` — the `[package]` block of `neotmux-layout/Cargo.toml` with `name = "neotmux-sys"`, then:

```toml
[dependencies]
neotmux-proto.workspace = true
anyhow.workspace = true
nix.workspace = true
signal-hook.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

(`tempfile` moves to `[dependencies]` only if `cargo check -p neotmux-sys` says a non-test item uses it.)

`neotmux-sys/src/lib.rs`: a `//!` doc (what sys is: what both processes need from the OS), then

```rust
pub mod link;
pub mod logging;
pub mod procinfo;
pub mod signals;
pub mod sock;
pub mod tty;
```

All six are `pub` because both other crates reach them; inside them, items that were `pub(crate)` and are now used from `neotmux-client` or `neotmux-server` become `pub` — each one only when the compiler names a cross-crate use. Rewrite inside sys: `crate::procinfo` stays `crate::procinfo` (same crate).

- [ ] **Step 2: The client crate**

```bash
mkdir -p neotmux-client/src
git mv neotmux/src/client/mod.rs neotmux-client/src/lib.rs
for f in neotmux/src/client/*.rs; do git mv "$f" neotmux-client/src/; done
rmdir neotmux/src/client
```

`neotmux-client/Cargo.toml`: the same `[package]` block with `name = "neotmux-client"`, and

```toml
[dependencies]
neotmux-proto.workspace = true
neotmux-sys.workspace = true
anyhow.workspace = true
libc.workspace = true
nix.workspace = true
ratatui.workspace = true
```

Rewrite paths inside `neotmux-client/src/` (Task 4 mapping table): `crate::client::` → `crate::`, `crate::{sock,link,signals,tty,logging,procinfo}` → `neotmux_sys::…`:

```bash
sed -i 's/crate::client::/crate::/g; s/crate::\(sock\|link\|signals\|tty\|logging\|procinfo\)\b/neotmux_sys::\1/g' neotmux-client/src/*.rs
```

Modules `main.rs` calls (`attach`, `command`, `kill`, `ls` — whatever `neotmux::client::X` names in `main.rs`) must be reachable from outside: widen them to `pub` only as far as `main.rs` needs.

- [ ] **Step 3: The server crate**

```bash
mkdir -p neotmux-server/src neotmux-server/tests
git mv neotmux/src/server/mod.rs neotmux-server/src/lib.rs
for f in neotmux/src/server/*.rs; do git mv "$f" neotmux-server/src/; done
rmdir neotmux/src/server
for t in headless key_spec_contract job_process; do git mv neotmux/tests/$t.rs neotmux-server/tests/$t.rs; done
```

`neotmux-server/Cargo.toml`: `[package]` with `name = "neotmux-server"`, and

```toml
[dependencies]
neotmux-proto.workspace = true
neotmux-layout.workspace = true
neotmux-pty.workspace = true
neotmux-lua.workspace = true
neotmux-sys.workspace = true
anyhow.workspace = true
nix.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

plus whatever the three moved tests `use` (check their `use` lines: `nix`, `tempfile`, `portable-pty`, `vt100` → dev-dependencies as named).

```bash
sed -i 's/crate::server::/crate::/g; s/crate::\(sock\|link\|signals\|tty\|logging\|procinfo\)\b/neotmux_sys::\1/g' neotmux-server/src/*.rs
sed -i 's/neotmux::server::/neotmux_server::/g' neotmux-server/tests/*.rs
```

If a moved test used `mod harness;` or `mod session;` from `neotmux/tests/`, it drives the binary after all: move it back to `neotmux/tests/` and give `neotmux` a dev-dependency on `neotmux-server` instead, and ledger that.

- [ ] **Step 4: The binary**

Root `Cargo.toml` `[workspace.dependencies]` gains:

```toml
neotmux-sys = { path = "neotmux-sys", version = "0.1.0" }
neotmux-client = { path = "neotmux-client", version = "0.1.0" }
neotmux-server = { path = "neotmux-server", version = "0.1.0" }
```

`neotmux/Cargo.toml` `[dependencies]` becomes:

```toml
neotmux-proto.workspace = true
neotmux-sys.workspace = true
neotmux-client.workspace = true
neotmux-server.workspace = true
anyhow.workspace = true
```

and keeps its `[dev-dependencies]`, adding `neotmux-client` / `neotmux-server` / `neotmux-pty` / `neotmux-lua` only where a remaining test in `neotmux/tests/` names them (`key_round_trip.rs` names both halves).

```bash
git rm neotmux/src/lib.rs
sed -i 's/neotmux::client::/neotmux_client::/g; s/neotmux::server::/neotmux_server::/g; s/neotmux::sock::/neotmux_sys::sock::/g; s/neotmux::logging/neotmux_sys::logging/g' neotmux/src/main.rs neotmux/tests/*.rs neotmux/tests/*/*.rs
```

- [ ] **Step 5: Compile until clean**

```bash
cargo check --workspace --all-targets 2>&1 | grep -E "^error" -A6 | head -60
```

Fix each error by the Task 4 mapping table, or by widening one item's visibility to `pub` where the error is "private" and the caller is in another crate. Repeat until clean. Nothing else changes.

- [ ] **Step 6: Prove the graph**

```bash
cargo tree -p neotmux-client -e normal --prefix none | sort -u | grep neotmux
cargo tree -p neotmux-layout -e normal --prefix none | sort -u | grep neotmux
```

Expected: the first lists only `neotmux-client`, `neotmux-proto`, `neotmux-sys`; the second only `neotmux-layout`, `neotmux-proto`.

- [ ] **Step 7: Check and commit**

Run the count. Expected: baseline.

```bash
git add -A
git commit -F - <<'EOF'
the client and the server are crates of their own, and what they share is sys

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 5: every lib.rs and mod.rs is a list of exports

One commit per crate, in this order (each is independent of the others except through paths): proto (5a), pty (5b), lua (5c), client and server (5d). `neotmux-layout` and `neotmux-sys` already satisfy the rule (their `lib.rs` is `mod` lines only after Task 4) — confirm and move on.

**The rule for a directory** `d/` holding files moved in from the crate root:

```rust
// d/mod.rs — for each file x that was `pub mod x;` at the root:
mod x;
pub use x::*;
// for each file y that was a private `mod y;` at the root:
mod y;
pub(crate) use y::*;
```

and the crate's `lib.rs` declares `pub mod d;` (or `mod d;` if every file in it was private). If `cargo check` reports an ambiguous glob name, that one file becomes `pub mod x;` in `d/mod.rs` instead of being flattened, and the ledger records which.

#### Task 5a: neotmux-proto

- [ ] **Step 1: Move**

```bash
cd neotmux-proto/src
mkdir screen geom wire
git mv draw.rs frame.rs read.rs width.rs screen/
git mv geom.rs geom/size.rs
git mv geometry.rs panerects.rs reserve.rs grid.rs geom/
git mv msg.rs wire/
git mv wire.rs wire/codec.rs
cd ../..
```

`screen/mod.rs`, `geom/mod.rs`, `wire/mod.rs` by the rule (all were `pub mod`). `PROTOCOL_VERSION` and its doc comment move from `lib.rs` to a new `version.rs`; `lib.rs` becomes:

```rust
//! (the crate doc, unchanged)
pub mod event;
pub mod geom;
pub mod ids;
pub mod keys;
pub mod level;
pub mod paneinfo;
pub mod screen;
pub mod version;
pub mod wire;

pub use event::Event;
pub use geom::{Axis, Color, Dir, Edge, Geometry, GridSnapshot, MIN_GRID, PaneRects, Reserved, Size, SnapCell, SplitDir, Window, clamp_to_min_grid};
pub use ids::{PaneId, SessionId, WindowId};
pub use keys::{Key, KeyCode, Modifiers};
pub use level::Level;
pub use paneinfo::{LineMode, PaneInfo, PaneTty};
pub use screen::{DrawCmd, Epoch, Extent, Frame, Lines, Notice, OwnedFrame, OwnedPaneFrame, PaneFrame, PaneRead, Rect, Trailing, Want};
pub use version::PROTOCOL_VERSION;
pub use wire::{OwnedToClient, ToClient, ToServer, WireError};
```

— the same names the root re-exported before Task 5 (plus Task 3's two), grouped by their new module.

- [ ] **Step 2: Rewrite paths and compile**

Inside the crate, `crate::draw::` etc. → `crate::screen::` by the Task 5 table; `crate::wire::` stays `crate::wire::` (the directory has the old module's name); `crate::geom::` stays. Outside:

```bash
grep -rl 'neotmux_proto::\(draw\|frame\|read\|width\|geometry\|panerects\|reserve\|grid\|msg\)::' --include=*.rs . | grep -v target | xargs sed -i 's/neotmux_proto::\(draw\|frame\|read\|width\)::/neotmux_proto::screen::/g; s/neotmux_proto::\(geometry\|panerects\|reserve\|grid\)::/neotmux_proto::geom::/g; s/neotmux_proto::msg::/neotmux_proto::wire::/g'
sed -i 's/crate::\(draw\|frame\|read\|width\)::/crate::screen::/g; s/crate::\(geometry\|panerects\|reserve\|grid\)::/crate::geom::/g; s/crate::msg::/crate::wire::/g' $(find neotmux-proto/src -name '*.rs')
cargo check --workspace --all-targets 2>&1 | grep -E "^error" -A6 | head -60
```

Also `neotmux_proto::draw` / `use neotmux_proto::{draw, …}` forms naming the module itself: fix those the compiler reports by the same table. Repeat until clean.

- [ ] **Step 3: Check and commit** — count = baseline.

```bash
git add -A
git commit -F - <<'EOF'
proto is four directories and the handful of words both ends share

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

#### Task 5b: neotmux-pty

- [ ] **Step 1: Move**

```bash
cd neotmux-pty/src
mkdir child screen parse
git mv pty.rs wakeup.rs child/
git mv state.rs scrollback.rs evictions.rs lines.rs wrap.rs screen/
git mv query.rs hvp.rs parse/
cd ../..
```

`child/mod.rs`, `screen/mod.rs` by the rule (all `pub`); `parse/mod.rs`: `mod query; pub use query::*; mod hvp; pub(crate) use hvp::*;` (`hvp` was private). `lib.rs`:

```rust
pub mod child;
pub mod parse;
pub mod screen;
pub mod terminals;
```

- [ ] **Step 2: Rewrite and compile**

```bash
grep -rl 'neotmux_pty::\(pty\|wakeup\|state\|scrollback\|evictions\|lines\|wrap\|query\)::' --include=*.rs . | grep -v target | xargs sed -i 's/neotmux_pty::\(pty\|wakeup\)::/neotmux_pty::child::/g; s/neotmux_pty::\(state\|scrollback\|evictions\|lines\|wrap\)::/neotmux_pty::screen::/g; s/neotmux_pty::query::/neotmux_pty::parse::/g'
sed -i 's/crate::\(pty\|wakeup\)::/crate::child::/g; s/crate::\(state\|scrollback\|evictions\|lines\|wrap\)::/crate::screen::/g; s/crate::\(query\|hvp\)::/crate::parse::/g' $(find neotmux-pty/src -name '*.rs')
cargo check --workspace --all-targets 2>&1 | grep -E "^error" -A6 | head -60
```

Repeat until clean.

- [ ] **Step 3: Check and commit** — count = baseline.

```bash
git add -A
git commit -F - <<'EOF'
the pty crate is the child, the screen, and either side of the parser

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

#### Task 5c: neotmux-lua

- [ ] **Step 1: Move**

```bash
cd neotmux-lua/src
mkdir host sched draw plugin
git mv ambient.rs ops.rs paneinfo.rs window.rs clipboard.rs runner.rs host/
git mv jobs.rs timers.rs budget.rs overrun.rs sched/
git mv draw.rs draw/buffers.rs
git mv float.rs reserve.rs draw/
git mv registry.rs source.rs commands.rs runtime.rs vm.rs plugin/
git mv lib.rs subsystem.rs
cd ../..
```

`mod.rs` for each by the rule — private before (`mod ambient; mod float; mod paneinfo;`) → `pub(crate) use`, the rest `pub use`.

`neotmux-lua/src/lib.rs` (new):

```rust
//! (the crate doc, moved from the head of subsystem.rs)
pub mod api;
pub mod draw;
pub mod host;
pub mod plugin;
pub mod sched;
mod subsystem;

pub use api::Opts;
pub use api::keymap;
pub use subsystem::*;
```

In `subsystem.rs`: delete the `mod …;` lines and the two `pub use api::…` lines (they are in `lib.rs` now); every `use x::Y` that named a sibling module relatively (`use ambient::Ambient;`, `use budget::{Budget, Outcome};`, …) becomes `use crate::<dir>::Y` by the Task 5 table (`use crate::host::Ambient;`, `use crate::sched::{Budget, Outcome};`).

- [ ] **Step 2: Rewrite and compile**

```bash
F=$(find neotmux-lua/src -name '*.rs')
sed -i 's/crate::\(ambient\|ops\|paneinfo\|window\|clipboard\|runner\)::/crate::host::/g; s/crate::\(jobs\|timers\|budget\|overrun\)::/crate::sched::/g; s/crate::\(float\|reserve\)::/crate::draw::/g; s/crate::\(registry\|source\|commands\|runtime\|vm\)::/crate::plugin::/g' $F
grep -rl 'neotmux_lua::\(ambient\|ops\|window\|clipboard\|runner\|jobs\|timers\|budget\|overrun\|reserve\|registry\|source\|commands\|runtime\|vm\|draw\)::' --include=*.rs . | grep -v target | xargs sed -i 's/neotmux_lua::\(ops\|window\|clipboard\|runner\)::/neotmux_lua::host::/g; s/neotmux_lua::\(jobs\|timers\|budget\|overrun\)::/neotmux_lua::sched::/g; s/neotmux_lua::reserve::/neotmux_lua::draw::/g; s/neotmux_lua::\(registry\|source\|commands\|runtime\|vm\)::/neotmux_lua::plugin::/g'
cargo check --workspace --all-targets 2>&1 | grep -E "^error" -A6 | head -60
```

(`crate::draw::` needs no rewrite: `draw` is both the old module and the new directory, and `Buffers` is flattened into it.) Fix what remains by the table; repeat until clean.

- [ ] **Step 3: Check and commit** — count = baseline.

```bash
git add -A
git commit -F - <<'EOF'
the lua crate says what it is in four directories, and its lib.rs only says where

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

#### Task 5d: neotmux-client and neotmux-server

- [ ] **Step 1: Implementation out of lib.rs**

For each of `neotmux-client` (→ `client.rs`) and `neotmux-server` (→ `server.rs`):

```bash
git mv neotmux-client/src/lib.rs neotmux-client/src/client.rs
git mv neotmux-server/src/lib.rs neotmux-server/src/server.rs
```

Then write a new `lib.rs` holding the crate doc (moved from the head of the implementation file), the `mod` / `pub mod` / `pub(crate) mod` lines moved out of the implementation file with their visibility unchanged, and `mod client; pub use client::*;` (resp. `mod server; pub use server::*;`). In the implementation file, relative `use sibling::X` becomes `use crate::sibling::X`.

- [ ] **Step 2: Compile, check, commit**

```bash
cargo check --workspace --all-targets 2>&1 | grep -E "^error" -A6 | head -60
```

Fix until clean; count = baseline.

```bash
git add -A
git commit -F - <<'EOF'
the client and the server say what they hold in their lib.rs and do the work elsewhere

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

- [ ] **Step 3: Confirm the rule everywhere**

```bash
for f in neotmux*/src/lib.rs $(find neotmux*/src -name mod.rs); do
  echo "== $f"; grep -vE '^\s*(//|$|(pub(\(crate\))? )?mod [a-z_]+;|(pub(\(crate\))? )?use [a-z_:{}, A-Z]+;)' "$f" | head -3
done
```

Expected: every file prints its `==` line and nothing else (multi-line `pub use x::{…}` lists may print their continuation lines — those are exports too and pass).

---

### Task 6: the docs say where things are

**Files:**
- Modify: `README.md`, `docs/plugins.md` (any crate or source path), `CHANGELOG.md` (Unreleased entry), `docs/superpowers/specs/2026-09-19-neotmux-design.md` (a note under the §2 table only)

- [ ] **Step 1: Find every stale path**

```bash
grep -rn 'neotmux-core\|neotmux-term\|crates/\|neotmux/src/\(client\|server\)/\|src/lib.rs' README.md docs/plugins.md NOTICE
```

Rewrite each to the current crate and path.

- [ ] **Step 2: The design doc's note**

Under the §2 crate table of `docs/superpowers/specs/2026-09-19-neotmux-design.md`, add:

```markdown
> Since 2026-09-25 (`2026-09-25-crate-layout-design.md`): `neotmux-core` is `neotmux-layout`,
> `neotmux-term` is `neotmux-pty`, and the `neotmux` crate is split into `neotmux-client`,
> `neotmux-server`, `neotmux-sys` and the binary. The table above is the design as written.
```

- [ ] **Step 3: Changelog**

Under `## Unreleased`, first entry:

```markdown
### The crates say what they are

The crates sit at the repository root: `neotmux-proto`, `neotmux-layout` (was `neotmux-core`),
`neotmux-pty` (was `neotmux-term`), `neotmux-lua`, and the old `neotmux` split into
`neotmux-client`, `neotmux-server`, `neotmux-sys` and the binary. The client depends on proto and
sys alone, so it cannot reach Lua, a PTY or the layout. Every `lib.rs` and `mod.rs` is a list of
exports, and files are grouped into directories by what they are for. Nothing behaves
differently.
```

- [ ] **Step 4: Check and commit** — count = baseline.

```bash
git add -A
git commit -F - <<'EOF'
the docs name the crates by what they are called now

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

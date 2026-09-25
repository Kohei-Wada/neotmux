# neotmux — crates you can read the shape of

Date: 2026-09-25
Status: proposed
Scope: the workspace's crates, their names, where they sit in the repository, and how each one's
`src/` is laid out. No behaviour changes; nothing here is visible from Lua or on the wire.

## 1. Goal and non-goals

### Goal

The dependency graph is already a clean DAG, and the drift `2026-09-23-crate-boundaries-design.md`
set out to close is closed. What is left is that the shape cannot be read off the tree:

- `neotmux-core`, `neotmux-term` and `neotmux-proto` do not say what they hold.
- `neotmux` is one crate holding a client, a server, and what the two share, so nothing but
  convention stops the client reaching the server's crates.
- Most `src/` directories are a flat run of files: 20 in `neotmux-lua`, 17 in `neotmux-proto`.
- `neotmux-lua/src/lib.rs` (1113 lines) and `neotmux/src/server/mod.rs` (1227 lines) are where
  the implementation lives, so the file that should say what a crate contains is the longest one
  in it.

This spec takes three conventions from yazi (`sxyazi/yazi`): crates side by side at the
repository root under one `members = ["neotmux-*"]` glob; one crate per role; and `lib.rs` /
`mod.rs` holding exports only, with directories as modules.

### Non-goals

- **One file per type.** yazi's other half of that rule — a file is a type, named after it — would
  split `wire.rs` (2119 lines) and others rather than move them. It is the next step, taken one
  crate at a time, each its own spec.
- **Behaviour.** Every commit moves files and rewrites paths. No function body changes except
  where a `use` path does.
- **A module macro.** yazi's `mod_flat!` is `mod x; pub use x::*;` spelled once. Written out, it
  needs no crate of its own.
- **Compatibility paths.** A module path that moves is rewritten at every use; nothing is
  re-exported under its old name.
- **The historical record.** Specs and plans already in `docs/superpowers/` name the paths they
  were written against and stay as they are.

## 2. The crates

| crate | role | holds | does not hold |
|---|---|---|---|
| `neotmux` | the binary: parses arguments, hands off to client or server | `main.rs`, the integration tests that drive the binary | anything with a body worth testing on its own |
| `neotmux-client` | the terminal's side: reads keys, paints the frames the server sends | attach, key decoding, drawing, overlays, starting a server (the re-exec), OSC 52 | sessions, panes, Lua |
| `neotmux-server` | the sessions' side: owns panes, their children and the Lua layer, and composes frames | session, shells, commands, key encoding, driving plugins, notifications | a terminal, drawing |
| `neotmux-sys` | what both processes need from the OS | the socket's place and bind (`sock`), the connection (`link`), signals, polling (`tty`), the log, `/proc` | anything neotmux-specific in state |
| `neotmux-lua` | where plugins run | the VM, the `neotmux.*` API, the budget, jobs and timers, floats and reservations | a PTY or a terminal, except through contracts the host satisfies |
| `neotmux-pty` (was `neotmux-term`) | one pane's inside | the PTY and the child's I/O, VT parsing, the screen and its scrollback | where the pane sits, drawing |
| `neotmux-layout` (was `neotmux-core`) | where panes sit | the pane tree, the window set, the one function that turns them into rectangles | a PTY (`PaneId` only), Lua |
| `neotmux-proto` | the vocabulary both ends share | wire messages and their encoding, keys, ids, geometry, draw commands, column width, the minimum grid | any dependency |

### 2.1 The graph

```
                        neotmux (bin)
                     ┌───────┴────────┐
                     ▼                ▼
             neotmux-client     neotmux-server
                     │        ┌───────┼──────────┬───────────┐
                     │        ▼       ▼          ▼           │
                     │   neotmux-lua  neotmux-pty  neotmux-layout
                     ▼        │       │          │           ▼
                neotmux-sys ◄─┼───────┼──────────┼────── (server)
                     │        │       │          │
                     ▼        ▼       ▼          ▼
                          neotmux-proto
```

| depends on → | proto | layout | pty | lua | sys | client | server |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| `neotmux` | ○ | | | | ○ | ○ | ○ |
| `neotmux-client` | ○ | | | | ○ | | |
| `neotmux-server` | ○ | ○ | ○ | ○ | ○ | | |
| `neotmux-sys` | ○ | | | | | | |
| `neotmux-lua` | ○ | | dev only | | | | |
| `neotmux-pty` | ○ | | | | | | |
| `neotmux-layout` | ○ | | | | | | |

What the compiler now holds that only convention held before: the client cannot reach Lua, a
PTY or the layout; Lua cannot reach a PTY; and client and server cannot reach each other — they
meet only through `neotmux-sys::link` carrying `neotmux-proto`'s types. The `neotmux` crate's
own edges to `neotmux-sys` and `neotmux-proto` are what `main.rs` reads today: `logging`,
`sock::DEFAULT_SESSION`, `sock::validate_session_name` and `PROTOCOL_VERSION`.

### 2.2 Layout in Lua, later

Issue #27 wants a Lua callback to stand in for `layout.rs`'s `assign`. The seam — call the
callback, fall back to `neotmux-layout`'s `assign` when it fails — belongs in `neotmux-server`,
which already depends on both. The one constraint this spec sets for it: **`neotmux-layout` never
depends on `neotmux-lua`.**

### 2.3 What moves to make the graph true

- `clamp_to_min_grid` and `MIN_GRID` move from `neotmux-term::state` to `neotmux-proto`'s grid
  module. Client and server both apply the minimum; the client should not reach the PTY crate for
  it.
- `osc52::set` moves from `neotmux-term` to `neotmux-client`, its only caller.
- The two `#[cfg(test)]` round trips between key decoding (client) and key encoding (server) —
  `client/decode.rs:429` and `server/input.rs:409` — move to an integration test in `neotmux`,
  the one crate that may see both.
- `crates/neotmux/tests/` files that `use neotmux::server::…` are about the server; they move to
  `neotmux-server/tests/` if they drive no binary, and stay in `neotmux/tests/` with
  `neotmux-server` as a dev-dependency if they do.

## 3. Inside each crate

`lib.rs` and every `mod.rs` hold `mod` lines and `pub use` lines, nothing else. A directory's
`mod.rs` re-exports its files flat (`mod x; pub use x::*;`), so a path is crate, directory, item:
`neotmux_proto::screen::DrawCmd`. What was implementation in `lib.rs` or `mod.rs` moves to a file
named after the crate or directory (`subsystem.rs`, `client.rs`, `server.rs`).

```
Cargo.toml               members = ["neotmux-*"]
neotmux/                 main.rs, tests/
neotmux-client/src/      lib.rs  client.rs
                         ask attach decode draw overlay start osc52
neotmux-server/src/      lib.rs  server.rs
                         clip command input notify paneinfo plugins runner session shells
neotmux-sys/src/         lib.rs  sock link signals tty logging procinfo
neotmux-lua/src/         lib.rs  subsystem.rs
                         api/     (as it is)
                         host/    ambient ops paneinfo window clipboard runner   — the host's contracts
                         sched/   jobs timers budget overrun                     — when what is called
                         draw/    buffers(was draw) float reserve                — what goes on screen
                         plugin/  registry source commands runtime vm            — where plugins come from
neotmux-pty/src/         lib.rs  terminals
                         child/   pty wakeup                                     — the child's I/O
                         screen/  state; and named: scrollback evictions lines wrap — the screen and its history
                         parse/   query hvp                                      — either side of the parser
neotmux-layout/src/      lib.rs  layout panes windows   (since split per type: 2026-09-25-layout-one-file-per-type-design.md)
neotmux-proto/src/       lib.rs  version.rs (PROTOCOL_VERSION)
                         wire/    msg codec(was wire)
                         geom/    size(was geom) geometry panerects reserve grid
                         screen/  draw frame read width
                         ids keys event level paneinfo
```

Three files are renamed on the way in, because a file named after the directory it sits in is
`clippy::module_inception`, which `-D warnings` refuses: `neotmux-proto`'s `wire.rs` becomes
`wire/codec.rs` (it is the encoders and decoders), its `geom.rs` becomes `geom/size.rs` (`Size`,
`SplitDir`, `Dir`), and `neotmux-lua`'s `draw.rs` becomes `draw/buffers.rs` (`Buffers`).

A module that was private stays private under the directory: `mod x; pub(crate) use x::*;`, so
flattening never widens what a crate exports. Two files in one directory exporting the same name
is an error only where the name is used; if one arises, that file stays a named `pub mod x;` in its
directory rather than being flattened, and the plan's ledger records which.

`neotmux-pty`'s `screen/` keeps four of its five files named (`screen::scrollback::lines`,
`screen::evictions::Feed`, `screen::lines::visible`, `screen::wrap::visible`): they are modules of
free functions with generic names (`lines`, `len`, `walk`, `visible`), which say what they are
only with the file in front of them, and `lines` is both a function in one and the name of
another.

## 4. Order of work

One commit per step, each green under `make check`, each moving files with `git mv` so history
follows them:

1. Crates to the repository root, workspace to the glob. Paths in `Cargo.toml`, `Makefile`, CI and
   `env!("CARGO_MANIFEST_DIR")`-relative runtime paths follow.
2. `neotmux-core` → `neotmux-layout`, `neotmux-term` → `neotmux-pty`: crate names and every `use`.
3. The small moves of §2.3.
4. `neotmux` split into `neotmux-sys`, `neotmux-client`, `neotmux-server` and the binary.
5. One commit per crate for §3's inside layout.

Then `README.md`, `docs/plugins.md`, `CHANGELOG.md` (an Unreleased entry) and the design doc's
§2 table are brought up to date — the design doc as a note under its table that names the new
crates, since its body is the record of what was designed.

## 5. Testing

Nothing new is tested: this is a move. The evidence is `make check` (fmt, clippy `-D warnings`,
the whole workspace suite) passing at every commit with the same test count as before, and
`cargo tree -p neotmux-client` showing no `neotmux-lua`, `neotmux-pty` or `neotmux-layout`.

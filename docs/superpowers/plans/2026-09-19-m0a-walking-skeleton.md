# neotmux M0a — Walking Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `neotmux` spawns `$SHELL` on a PTY, renders it full-screen, forwards keystrokes, and exits cleanly — leaving the terminal in a sane state on every path including panic.

**Architecture:** Three crates. `neotmux-term` owns the PTY and terminal state (a thin wrapper over `vt100`, which supplies scrollback and per-cell access so no VT implementation is written here). `neotmux-core` holds the boundary types in a `proto` module — types only, no logic. The `neotmux` binary owns the main loop, input encoding and rendering. Because `portable-pty` is entirely blocking and crossterm's `event::poll` cannot watch an external fd, PTY reads happen on a dedicated thread that sends `Vec<u8>` over a channel; everything else stays on the main thread, which is where Lua will live from M1a.

**Tech Stack:** Rust 1.95, `vt100` 0.16, `portable-pty` 0.9, `ratatui` 0.30 (crossterm backend), `crossterm` 0.29, `anyhow` 1.

**Spec:** `docs/superpowers/specs/2026-09-19-neotmux-design.md`

## Global Constraints

- Edition 2024, rustc 1.95 or newer. Workspace with a virtual manifest; dependency versions are declared once in `[workspace.dependencies]` and referenced by members with `workspace = true`
- Exact dependency versions: `vt100 = "0.16"`, `portable-pty = "0.9"`, `ratatui = "0.30"`, `crossterm = "0.29"`, `anyhow = "1"`
- `neotmux-core` must compile and test with no terminal and no PTY. If a test there needs either, a boundary has leaked
- `neotmux-core::proto` contains types only; the sole permitted logic is `From` / `Display` style conversion
- **Invariant: no code path may leave the terminal in raw mode or in the alternate screen.** This includes panics
- Indentation is spaces, never tabs. No code comments unless the *why* is non-obvious, and then one line
- Every task ends with `cargo test --workspace`, `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` passing, then a commit

---

## File Structure

| path | responsibility |
|---|---|
| `Cargo.toml` | virtual workspace manifest, shared dependency versions |
| `crates/neotmux-core/src/lib.rs` | crate root; `pub mod proto;` |
| `crates/neotmux-core/src/proto/mod.rs` | re-exports of the boundary types |
| `crates/neotmux-core/src/proto/ids.rs` | `PaneId` |
| `crates/neotmux-core/src/proto/geom.rs` | `Size` |
| `crates/neotmux-core/src/proto/grid.rs` | `GridSnapshot`, `SnapCell`, `Color` |
| `crates/neotmux-term/src/lib.rs` | crate root; `pub mod pty; pub mod state;` |
| `crates/neotmux-term/src/state.rs` | `TermState` — `vt100::Parser` wrapper, `feed`, `resize`, `snapshot_into` |
| `crates/neotmux-term/src/pty.rs` | `PtyHandle` — spawn, writer, resize, reader thread + channel |
| `crates/neotmux-term/tests/shell_output.rs` | integration: run a real command on a PTY, assert the grid |
| `crates/neotmux/src/main.rs` | argument handling, terminal setup/teardown, calls `app::run` |
| `crates/neotmux/src/app.rs` | the main loop |
| `crates/neotmux/src/input.rs` | crossterm `KeyEvent` → PTY bytes |
| `crates/neotmux/src/render.rs` | `GridSnapshot` → ratatui buffer |
| `crates/neotmux/tests/terminal_restore.rs` | integration: run the binary inside a PTY, assert teardown on exit and on panic |

Split by responsibility: `state.rs` never touches a file descriptor, `pty.rs` never parses a byte, `render.rs` never reads a PTY, `input.rs` is a pure function. Each is independently testable, which is what keeps the M1a work from having to reshape them.

---

### Task 1: Workspace skeleton

**Files:**
- Create: `Cargo.toml`, `crates/neotmux-core/Cargo.toml`, `crates/neotmux-core/src/lib.rs`, `crates/neotmux-term/Cargo.toml`, `crates/neotmux-term/src/lib.rs`, `crates/neotmux/Cargo.toml`, `crates/neotmux/src/main.rs`
- Modify: `.gitignore` (already has `/target`; verify)

**Interfaces:**
- Consumes: nothing
- Produces: a buildable workspace with members `neotmux-core`, `neotmux-term`, `neotmux`

- [ ] **Step 1: Write the workspace manifest**

`Cargo.toml`:

```toml
[workspace]
members = ["crates/neotmux-core", "crates/neotmux-term", "crates/neotmux"]
resolver = "3"

[workspace.package]
edition = "2024"
rust-version = "1.95"
license = "MIT OR Apache-2.0"
repository = "https://github.com/Kohei-Wada/neotmux"

[workspace.dependencies]
neotmux-core = { path = "crates/neotmux-core" }
neotmux-term = { path = "crates/neotmux-term" }
anyhow = "1"
crossterm = "0.29"
portable-pty = "0.9"
ratatui = "0.30"
vt100 = "0.16"
```

- [ ] **Step 2: Write the member manifests**

`crates/neotmux-core/Cargo.toml`:

```toml
[package]
name = "neotmux-core"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
```

`crates/neotmux-term/Cargo.toml`:

```toml
[package]
name = "neotmux-term"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
neotmux-core.workspace = true
anyhow.workspace = true
portable-pty.workspace = true
vt100.workspace = true
```

`crates/neotmux/Cargo.toml`:

```toml
[package]
name = "neotmux"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
neotmux-core.workspace = true
neotmux-term.workspace = true
anyhow.workspace = true
crossterm.workspace = true
ratatui.workspace = true

[dev-dependencies]
portable-pty.workspace = true
```

- [ ] **Step 3: Write the crate roots**

`crates/neotmux-core/src/lib.rs`:

```rust
pub mod proto;
```

`crates/neotmux-core/src/proto/mod.rs` (placeholder, filled in Task 2):

```rust
```

`crates/neotmux-term/src/lib.rs`:

```rust
pub mod pty;
pub mod state;
```

Create empty `crates/neotmux-term/src/pty.rs` and `crates/neotmux-term/src/state.rs` so the crate compiles.

`crates/neotmux/src/main.rs`:

```rust
fn main() {
    println!("neotmux");
}
```

- [ ] **Step 4: Verify the workspace builds**

Run: `cargo build --workspace`
Expected: success, three crates compiled.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates
git commit -m "chore: cargo workspace skeleton"
```

---

### Task 2: Boundary types in `neotmux-core::proto`

**Files:**
- Create: `crates/neotmux-core/src/proto/ids.rs`, `crates/neotmux-core/src/proto/geom.rs`, `crates/neotmux-core/src/proto/grid.rs`
- Modify: `crates/neotmux-core/src/proto/mod.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `PaneId(pub u32)`, `impl Display`
  - `Size { rows: u16, cols: u16 }`
  - `Color { Default, Idx(u8), Rgb(u8, u8, u8) }` (mirrors `vt100::Color` so `neotmux-core` needs no vt100 dependency)
  - `SnapCell { text: String, fg: Color, bg: Color, bold: bool, italic: bool, underline: bool, inverse: bool, wide: bool, wide_continuation: bool }`, `impl Default`
  - `GridSnapshot { size: Size, cursor: (u16, u16), hide_cursor: bool, cells: Vec<SnapCell> }` with `new(size)`, `resize(&mut self, size)`, `cell(&self, row, col) -> Option<&SnapCell>`, `cell_mut(&mut self, row, col) -> Option<&mut SnapCell>`

`GridSnapshot` owns a flat row-major `Vec` and is **reused across frames** — `resize` keeps the allocation when the area is unchanged. This is why the producer API in Task 3 is `snapshot_into(&mut GridSnapshot)` rather than a function returning a fresh snapshot.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-core/src/proto/grid.rs` (test module at the bottom of the file you are about to create):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_row_major() {
        let mut g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        g.cell_mut(1, 2).unwrap().text.push('X');
        assert_eq!(g.cells[5].text, "X");
        assert_eq!(g.cell(1, 2).unwrap().text, "X");
    }

    #[test]
    fn rejects_out_of_bounds() {
        let g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        assert!(g.cell(2, 0).is_none());
        assert!(g.cell(0, 3).is_none());
    }

    #[test]
    fn resize_to_same_size_keeps_the_allocation() {
        let mut g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        let ptr = g.cells.as_ptr();
        g.resize(Size { rows: 2, cols: 3 });
        assert_eq!(g.cells.as_ptr(), ptr);
    }

    #[test]
    fn resize_clears_cells() {
        let mut g = GridSnapshot::new(Size { rows: 1, cols: 1 });
        g.cell_mut(0, 0).unwrap().text.push('X');
        g.resize(Size { rows: 1, cols: 1 });
        assert_eq!(g.cell(0, 0).unwrap().text, "");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-core`
Expected: compile error — `GridSnapshot` not found.

- [ ] **Step 3: Write the types**

`crates/neotmux-core/src/proto/ids.rs`:

```rust
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(pub u32);

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}
```

`crates/neotmux-core/src/proto/geom.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    pub fn area(self) -> usize {
        usize::from(self.rows) * usize::from(self.cols)
    }
}
```

`crates/neotmux-core/src/proto/grid.rs` (above the test module written in Step 1):

```rust
use crate::proto::geom::Size;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapCell {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
    pub wide_continuation: bool,
}

impl SnapCell {
    pub fn reset(&mut self) {
        self.text.clear();
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.bold = false;
        self.italic = false;
        self.underline = false;
        self.inverse = false;
        self.wide = false;
        self.wide_continuation = false;
    }
}

#[derive(Debug, Clone, Default)]
pub struct GridSnapshot {
    pub size: Size,
    pub cursor: (u16, u16),
    pub hide_cursor: bool,
    pub cells: Vec<SnapCell>,
}

impl GridSnapshot {
    pub fn new(size: Size) -> Self {
        let mut this = Self::default();
        this.resize(size);
        this
    }

    pub fn resize(&mut self, size: Size) {
        self.size = size;
        self.cells.resize(size.area(), SnapCell::default());
        for cell in &mut self.cells {
            cell.reset();
        }
    }

    fn index(&self, row: u16, col: u16) -> Option<usize> {
        if row >= self.size.rows || col >= self.size.cols {
            return None;
        }
        Some(usize::from(row) * usize::from(self.size.cols) + usize::from(col))
    }

    pub fn cell(&self, row: u16, col: u16) -> Option<&SnapCell> {
        self.index(row, col).map(|i| &self.cells[i])
    }

    pub fn cell_mut(&mut self, row: u16, col: u16) -> Option<&mut SnapCell> {
        self.index(row, col).map(|i| &mut self.cells[i])
    }
}
```

`crates/neotmux-core/src/proto/mod.rs`:

```rust
pub mod geom;
pub mod grid;
pub mod ids;

pub use geom::Size;
pub use grid::{Color, GridSnapshot, SnapCell};
pub use ids::PaneId;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-core`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-core
git commit -m "feat(core): boundary types for ids, geometry and grid snapshots"
```

---

### Task 3: `TermState` — terminal state over `vt100`

**Files:**
- Modify: `crates/neotmux-term/src/state.rs`

**Interfaces:**
- Consumes: `neotmux_core::proto::{Color, GridSnapshot, Size, SnapCell}`
- Produces:
  - `TermState::new(size: Size, scrollback: usize) -> TermState`
  - `TermState::feed(&mut self, bytes: &[u8])`
  - `TermState::resize(&mut self, size: Size)`
  - `TermState::size(&self) -> Size`
  - `TermState::snapshot_into(&self, out: &mut GridSnapshot)`

`vt100`'s API, confirmed against 0.16.2: `Parser::new(rows, cols, scrollback_len)`, `Parser::process(&[u8])`, `Parser::screen() -> &Screen`, `Screen::set_size(rows, cols)`, `Screen::size() -> (u16, u16)`, `Screen::cell(row, col) -> Option<&Cell>`, `Screen::cursor_position() -> (u16, u16)`, `Screen::hide_cursor() -> bool`, `Cell::contents() -> &str`, `Cell::{fgcolor,bgcolor} -> vt100::Color`, `Cell::{bold,italic,underline,inverse,is_wide,is_wide_continuation}`.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux-term/src/state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: &TermState) -> GridSnapshot {
        let mut out = GridSnapshot::default();
        state.snapshot_into(&mut out);
        out
    }

    #[test]
    fn plain_text_lands_in_the_grid() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.feed(b"hi");
        let g = snap(&s);
        assert_eq!(g.cell(0, 0).unwrap().text, "h");
        assert_eq!(g.cell(0, 1).unwrap().text, "i");
        assert_eq!(g.cursor, (0, 2));
    }

    #[test]
    fn sgr_bold_and_indexed_colour_are_carried() {
        let mut s = TermState::new(Size { rows: 1, cols: 4 }, 0);
        s.feed(b"\x1b[1;31mR\x1b[0m");
        let c = snap(&s);
        let cell = c.cell(0, 0).unwrap();
        assert_eq!(cell.text, "R");
        assert!(cell.bold);
        assert_eq!(cell.fg, Color::Idx(1));
    }

    #[test]
    fn truecolour_is_carried() {
        let mut s = TermState::new(Size { rows: 1, cols: 2 }, 0);
        s.feed(b"\x1b[38;2;10;20;30mZ");
        assert_eq!(snap(&s).cell(0, 0).unwrap().fg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn cursor_positioning_and_erase_work() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.feed(b"abcd\x1b[1;1H\x1b[K");
        let g = snap(&s);
        assert_eq!(g.cell(0, 0).unwrap().text, "");
        assert_eq!(g.cursor, (0, 0));
    }

    #[test]
    fn resize_is_reflected_in_the_snapshot() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.resize(Size { rows: 3, cols: 6 });
        assert_eq!(s.size(), Size { rows: 3, cols: 6 });
        assert_eq!(snap(&s).size, Size { rows: 3, cols: 6 });
    }

    #[test]
    fn snapshot_into_reuses_the_buffer() {
        let s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        let mut out = GridSnapshot::default();
        s.snapshot_into(&mut out);
        let ptr = out.cells.as_ptr();
        s.snapshot_into(&mut out);
        assert_eq!(out.cells.as_ptr(), ptr);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-term`
Expected: compile error — `TermState` not found.

- [ ] **Step 3: Write the implementation**

At the top of `crates/neotmux-term/src/state.rs`:

```rust
use neotmux_core::proto::{Color, GridSnapshot, Size};

pub struct TermState {
    parser: vt100::Parser,
}

fn color_from_vt100(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(i) => Color::Idx(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

impl TermState {
    pub fn new(size: Size, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new(size.rows, size.cols, scrollback),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, size: Size) {
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }

    pub fn size(&self) -> Size {
        let (rows, cols) = self.parser.screen().size();
        Size { rows, cols }
    }

    pub fn snapshot_into(&self, out: &mut GridSnapshot) {
        let screen = self.parser.screen();
        let size = self.size();
        out.resize(size);
        out.cursor = screen.cursor_position();
        out.hide_cursor = screen.hide_cursor();
        for row in 0..size.rows {
            for col in 0..size.cols {
                let Some(src) = screen.cell(row, col) else {
                    continue;
                };
                let dst = out.cell_mut(row, col).expect("in bounds");
                dst.text.push_str(src.contents());
                dst.fg = color_from_vt100(src.fgcolor());
                dst.bg = color_from_vt100(src.bgcolor());
                dst.bold = src.bold();
                dst.italic = src.italic();
                dst.underline = src.underline();
                dst.inverse = src.inverse();
                dst.wide = src.is_wide();
                dst.wide_continuation = src.is_wide_continuation();
            }
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-term`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-term/src/state.rs
git commit -m "feat(term): terminal state over vt100 with reusable grid snapshots"
```

---

### Task 4: `PtyHandle` — PTY plus reader thread

**Files:**
- Modify: `crates/neotmux-term/src/pty.rs`
- Create: `crates/neotmux-term/tests/shell_output.rs`

**Interfaces:**
- Consumes: `neotmux_core::proto::Size`, `TermState` (in the integration test only)
- Produces:
  - `PtyHandle::spawn(cmd: CommandBuilder, size: Size) -> anyhow::Result<PtyHandle>`
  - `PtyHandle::default_shell_command() -> CommandBuilder`
  - `PtyHandle::output(&self) -> &Receiver<PtyEvent>`
  - `PtyHandle::write(&mut self, bytes: &[u8]) -> io::Result<()>`
  - `PtyHandle::resize(&self, size: Size) -> anyhow::Result<()>`
  - `PtyHandle::wait(&mut self) -> io::Result<ExitStatus>`
  - `enum PtyEvent { Output(Vec<u8>), Eof }`

Facts fixed by `portable-pty` 0.9: `take_writer()` may be called only once, so the writer is taken at construction and stored. `try_clone_reader()` yields a blocking `Read + Send`, which the reader thread owns. PTY EOF arrives as `Ok(0)` (the crate maps `EIO` to it), which the thread turns into `PtyEvent::Eof` before exiting.

- [ ] **Step 1: Write the failing integration test**

`crates/neotmux-term/tests/shell_output.rs`:

```rust
use std::time::{Duration, Instant};

use neotmux_core::proto::{GridSnapshot, Size};
use neotmux_term::pty::{PtyEvent, PtyHandle};
use neotmux_term::state::TermState;
use portable_pty::CommandBuilder;

#[test]
fn a_command_running_on_the_pty_reaches_the_grid() {
    let size = Size { rows: 24, cols: 80 };
    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", "printf X"]);
    let mut pty = PtyHandle::spawn(cmd, size).expect("spawn");

    let mut state = TermState::new(size, 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match pty.output().recv_timeout(Duration::from_millis(250)) {
            Ok(PtyEvent::Output(bytes)) => state.feed(&bytes),
            Ok(PtyEvent::Eof) => break,
            Err(_) if Instant::now() < deadline => continue,
            Err(e) => panic!("timed out waiting for pty output: {e}"),
        }
    }

    let mut grid = GridSnapshot::default();
    state.snapshot_into(&mut grid);
    assert_eq!(grid.cell(0, 0).unwrap().text, "X");

    let status = pty.wait().expect("wait");
    assert!(status.success());
}

#[test]
fn writing_to_the_pty_reaches_the_child() {
    let size = Size { rows: 24, cols: 80 };
    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", "read line; printf '[%s]' \"$line\""]);
    let mut pty = PtyHandle::spawn(cmd, size).expect("spawn");
    pty.write(b"ok\n").expect("write");

    let mut state = TermState::new(size, 0);
    loop {
        match pty.output().recv_timeout(Duration::from_secs(10)) {
            Ok(PtyEvent::Output(bytes)) => state.feed(&bytes),
            Ok(PtyEvent::Eof) | Err(_) => break,
        }
    }

    let mut grid = GridSnapshot::default();
    state.snapshot_into(&mut grid);
    let line: String = (0..size.cols)
        .filter_map(|c| grid.cell(0, c).map(|cell| cell.text.clone()))
        .collect();
    assert!(line.contains("[ok]"), "got {line:?}");

    pty.wait().expect("wait");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p neotmux-term --test shell_output`
Expected: compile error — `neotmux_term::pty::PtyHandle` not found.

- [ ] **Step 3: Write the implementation**

`crates/neotmux-term/src/pty.rs`:

```rust
use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use anyhow::Context as _;
use neotmux_core::proto::Size;
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

pub enum PtyEvent {
    Output(Vec<u8>),
    Eof,
}

pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<PtyEvent>,
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl PtyHandle {
    pub fn default_shell_command() -> CommandBuilder {
        CommandBuilder::new_default_prog()
    }

    pub fn spawn(cmd: CommandBuilder, size: Size) -> anyhow::Result<Self> {
        let pair = native_pty_system()
            .openpty(pty_size(size))
            .context("open pty")?;
        let child = pair.slave.spawn_command(cmd).context("spawn command")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;
        let (tx, output) = channel();
        thread::Builder::new()
            .name("neotmux-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => {
                            let _ = tx.send(PtyEvent::Eof);
                            return;
                        }
                        Ok(n) => {
                            if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                                return;
                            }
                        }
                    }
                }
            })
            .context("spawn reader thread")?;

        Ok(Self {
            master: pair.master,
            writer,
            child,
            output,
        })
    }

    pub fn output(&self) -> &Receiver<PtyEvent> {
        &self.output
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&self, size: Size) -> anyhow::Result<()> {
        self.master.resize(pty_size(size)).context("resize pty")
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-term`
Expected: 8 passed (6 unit, 2 integration).

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-term
git commit -m "feat(term): pty handle with a blocking reader thread"
```

---

### Task 5: Rendering a snapshot into a ratatui buffer

**Files:**
- Create: `crates/neotmux/src/render.rs`
- Modify: `crates/neotmux/src/main.rs` (declare `mod render;`)

**Interfaces:**
- Consumes: `neotmux_core::proto::{Color, GridSnapshot}`
- Produces: `render::draw_grid(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, grid: &GridSnapshot)`

`Color` is mapped as: `Default` → `ratatui::style::Color::Reset`, `Idx(i)` → `Indexed(i)`, `Rgb(r,g,b)` → `Rgb(r,g,b)`. A cell whose `text` is empty renders as a space. `wide_continuation` cells are skipped — the wide cell before them already occupies the column.

- [ ] **Step 1: Write the failing test**

At the bottom of `crates/neotmux/src/render.rs`:

```rust
#[cfg(test)]
mod tests {
    use neotmux_core::proto::{Color as PColor, GridSnapshot, Size};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    use super::draw_grid;

    fn render(grid: &GridSnapshot, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| draw_grid(frame, frame.area(), grid))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn writes_characters_at_their_coordinates() {
        let mut g = GridSnapshot::new(Size { rows: 2, cols: 3 });
        g.cell_mut(0, 0).unwrap().text.push('X');
        g.cell_mut(1, 2).unwrap().text.push('Y');
        let buf = render(&g, 3, 2);
        assert_eq!(buf[(0, 0)].symbol(), "X");
        assert_eq!(buf[(2, 1)].symbol(), "Y");
        assert_eq!(buf[(1, 0)].symbol(), " ");
    }

    #[test]
    fn carries_colour_and_bold() {
        let mut g = GridSnapshot::new(Size { rows: 1, cols: 1 });
        let cell = g.cell_mut(0, 0).unwrap();
        cell.text.push('R');
        cell.fg = PColor::Idx(1);
        cell.bg = PColor::Rgb(1, 2, 3);
        cell.bold = true;
        let buf = render(&g, 1, 1);
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(1));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(1, 2, 3));
        assert!(buf[(0, 0)].modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn clips_to_the_area() {
        let mut g = GridSnapshot::new(Size { rows: 4, cols: 4 });
        g.cell_mut(3, 3).unwrap().text.push('Z');
        let buf = render(&g, 2, 2);
        assert_eq!(buf.area().width, 2);
        assert_eq!(buf.area().height, 2);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p neotmux`
Expected: compile error — `draw_grid` not found.

- [ ] **Step 3: Write the implementation**

At the top of `crates/neotmux/src/render.rs`:

```rust
use neotmux_core::proto::{Color as PColor, GridSnapshot};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};

fn color(c: PColor) -> Color {
    match c {
        PColor::Default => Color::Reset,
        PColor::Idx(i) => Color::Indexed(i),
        PColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub fn draw_grid(frame: &mut Frame, area: Rect, grid: &GridSnapshot) {
    let buf = frame.buffer_mut();
    let rows = grid.size.rows.min(area.height);
    let cols = grid.size.cols.min(area.width);

    for row in 0..rows {
        for col in 0..cols {
            let Some(src) = grid.cell(row, col) else {
                continue;
            };
            if src.wide_continuation {
                continue;
            }
            let Some(dst) = buf.cell_mut((area.x + col, area.y + row)) else {
                continue;
            };
            dst.set_symbol(if src.text.is_empty() { " " } else { &src.text });
            dst.set_fg(color(src.fg));
            dst.set_bg(color(src.bg));
            let mut modifier = Modifier::empty();
            modifier.set(Modifier::BOLD, src.bold);
            modifier.set(Modifier::ITALIC, src.italic);
            modifier.set(Modifier::UNDERLINED, src.underline);
            modifier.set(Modifier::REVERSED, src.inverse);
            dst.modifier = modifier;
        }
    }

    if !grid.hide_cursor {
        let (crow, ccol) = grid.cursor;
        if crow < area.height && ccol < area.width {
            frame.set_cursor_position(Position::new(area.x + ccol, area.y + crow));
        }
    }
}
```

`crates/neotmux/src/main.rs`:

```rust
mod render;

fn main() {
    println!("neotmux");
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux`
Expected: 3 passed.

If `buf.cell_mut((x, y))` or `buf[(x, y)]` does not resolve, check the signature with
`cargo doc -p ratatui --open` or `grep -rn "pub fn cell_mut" ~/.cargo/registry/src/*/ratatui-core-*/src/buffer/buffer.rs` and adjust the call, not the test's intent.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/src/render.rs crates/neotmux/src/main.rs
git commit -m "feat(neotmux): render a grid snapshot into a ratatui buffer"
```

---

### Task 6: Key encoding

**Files:**
- Create: `crates/neotmux/src/input.rs`
- Modify: `crates/neotmux/src/main.rs` (declare `mod input;`)

**Interfaces:**
- Consumes: `crossterm::event::KeyEvent`
- Produces: `input::encode_key(key: KeyEvent) -> Option<Vec<u8>>` — `None` for a key with no PTY representation

A pure function, which is why it is its own task and its own file: it has the highest test-to-code ratio in M0a and will be the first thing M1a's key-consumption contract wraps.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux/src/input.rs`:

```rust
#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::encode_key;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn plain_characters_are_utf8() {
        assert_eq!(encode_key(key(KeyCode::Char('a'))).unwrap(), b"a");
        assert_eq!(encode_key(key(KeyCode::Char('あ'))).unwrap(), "あ".as_bytes());
    }

    #[test]
    fn control_characters_are_masked() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(ctrl_c).unwrap(), vec![0x03]);
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(ctrl_d).unwrap(), vec![0x04]);
    }

    #[test]
    fn named_keys_map_to_their_bytes() {
        assert_eq!(encode_key(key(KeyCode::Enter)).unwrap(), b"\r");
        assert_eq!(encode_key(key(KeyCode::Tab)).unwrap(), b"\t");
        assert_eq!(encode_key(key(KeyCode::Backspace)).unwrap(), vec![0x7f]);
        assert_eq!(encode_key(key(KeyCode::Esc)).unwrap(), vec![0x1b]);
    }

    #[test]
    fn arrows_use_normal_cursor_mode() {
        assert_eq!(encode_key(key(KeyCode::Up)).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(key(KeyCode::Down)).unwrap(), b"\x1b[B");
        assert_eq!(encode_key(key(KeyCode::Right)).unwrap(), b"\x1b[C");
        assert_eq!(encode_key(key(KeyCode::Left)).unwrap(), b"\x1b[D");
    }

    #[test]
    fn alt_prefixes_with_escape() {
        let alt_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(encode_key(alt_a).unwrap(), b"\x1ba");
    }

    #[test]
    fn unsupported_keys_return_none() {
        assert!(encode_key(key(KeyCode::F(13))).is_none());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux --lib input`
Expected: compile error — `encode_key` not found.

- [ ] **Step 3: Write the implementation**

At the top of `crates/neotmux/src/input.rs`:

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn encode_key(key: KeyEvent) -> Option<Vec<u8>> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    let mut bytes = match key.code {
        KeyCode::Char(c) if ctrl => {
            let upper = c.to_ascii_uppercase();
            if upper.is_ascii() {
                vec![(upper as u8) & 0x1f]
            } else {
                return None;
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::F(n @ 1..=4) => vec![0x1b, b'O', b'P' + (n - 1)],
        KeyCode::F(n @ 5..=12) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                _ => 24,
            };
            format!("\x1b[{code}~").into_bytes()
        }
        _ => return None,
    };

    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}
```

`crates/neotmux/src/main.rs`:

```rust
mod input;
mod render;

fn main() {
    println!("neotmux");
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux`
Expected: 9 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux/src/input.rs crates/neotmux/src/main.rs
git commit -m "feat(neotmux): encode key events as pty bytes"
```

---

### Task 7: The main loop and terminal lifecycle

**Files:**
- Create: `crates/neotmux/src/app.rs`, `crates/neotmux/tests/terminal_restore.rs`
- Modify: `crates/neotmux/src/main.rs`

**Interfaces:**
- Consumes: `render::draw_grid`, `input::encode_key`, `neotmux_term::pty::{PtyEvent, PtyHandle}`, `neotmux_term::state::TermState`
- Produces: `app::run(terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()>`

The loop drains the PTY channel without blocking, waits for input for whatever remains of the frame budget, then renders. `ratatui::init()` installs a panic hook that restores the terminal before the message is printed, which is what makes the invariant hold on the panic path. The `NEOTMUX_PANIC_TEST` escape hatch exists only in debug builds and only so the invariant is testable.

- [ ] **Step 1: Write the failing integration test**

`crates/neotmux/tests/terminal_restore.rs`:

```rust
use std::io::Read;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn run_in_pty(env: &[(&str, &str)], input: &[u8]) -> (String, bool) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    if !input.is_empty() {
        std::thread::sleep(Duration::from_millis(500));
        use std::io::Write as _;
        writer.write_all(input).expect("write");
        writer.flush().expect("flush");
    }

    let collector = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        out
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            break s;
        }
        assert!(Instant::now() < deadline, "neotmux did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(pair.master);
    let out = collector.join().expect("collector");
    (String::from_utf8_lossy(&out).into_owned(), status.success())
}

#[test]
fn leaves_the_alternate_screen_on_clean_exit() {
    let (out, ok) = run_in_pty(&[("SHELL", "/bin/sh")], b"exit\r");
    assert!(ok, "expected a clean exit");
    assert!(
        out.contains("\x1b[?1049l"),
        "alternate screen was never left"
    );
}

#[test]
fn leaves_the_alternate_screen_on_panic() {
    let (out, ok) = run_in_pty(&[("SHELL", "/bin/sh"), ("NEOTMUX_PANIC_TEST", "1")], b"");
    assert!(!ok, "expected a failing exit");
    assert!(
        out.contains("\x1b[?1049l"),
        "alternate screen was never left"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p neotmux --test terminal_restore`
Expected: both tests fail — the binary prints `neotmux` and exits without ever entering the alternate screen.

- [ ] **Step 3: Write the main loop**

`crates/neotmux/src/app.rs`:

```rust
use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::event::{self, Event};
use neotmux_core::proto::{GridSnapshot, Size};
use neotmux_term::pty::{PtyEvent, PtyHandle};
use neotmux_term::state::TermState;
use ratatui::DefaultTerminal;

use crate::input::encode_key;
use crate::render::draw_grid;

const FRAME: Duration = Duration::from_millis(16);
const SCROLLBACK: usize = 10_000;

fn terminal_size(terminal: &DefaultTerminal) -> anyhow::Result<Size> {
    let area = terminal.size().context("terminal size")?;
    Ok(Size {
        rows: area.height,
        cols: area.width,
    })
}

pub fn run(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    let mut size = terminal_size(terminal)?;
    let mut state = TermState::new(size, SCROLLBACK);
    let mut pty = PtyHandle::spawn(PtyHandle::default_shell_command(), size)?;
    let mut grid = GridSnapshot::default();

    if cfg!(debug_assertions) && std::env::var_os("NEOTMUX_PANIC_TEST").is_some() {
        panic!("NEOTMUX_PANIC_TEST");
    }

    loop {
        let frame_end = Instant::now() + FRAME;

        let mut eof = false;
        while let Ok(ev) = pty.output().try_recv() {
            match ev {
                PtyEvent::Output(bytes) => state.feed(&bytes),
                PtyEvent::Eof => eof = true,
            }
        }
        if eof {
            break;
        }

        while Instant::now() < frame_end {
            let remaining = frame_end.saturating_duration_since(Instant::now());
            if !event::poll(remaining).context("poll input")? {
                break;
            }
            match event::read().context("read input")? {
                Event::Key(key) => {
                    if let Some(bytes) = encode_key(key) {
                        pty.write(&bytes).context("write to pty")?;
                    }
                }
                Event::Resize(cols, rows) => {
                    size = Size { rows, cols };
                    state.resize(size);
                    pty.resize(size)?;
                }
                _ => {}
            }
        }

        state.snapshot_into(&mut grid);
        terminal
            .draw(|frame| draw_grid(frame, frame.area(), &grid))
            .context("draw")?;
    }

    pty.wait().context("wait for child")?;
    Ok(())
}
```

`crates/neotmux/src/main.rs`:

```rust
mod app;
mod input;
mod render;

fn main() -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal);
    ratatui::restore();
    result
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux`
Expected: all pass, including both `terminal_restore` tests.

If `leaves_the_alternate_screen_on_panic` fails, check that `ratatui::init()` (not a hand-rolled setup) is used — its panic hook is what restores the terminal before the unwind.

- [ ] **Step 5: Verify it interactively**

Run: `cargo run -p neotmux`
Expected: a working shell. Type `ls`, resize the window, then `exit`. After exiting, run `stty -a | head -1` and confirm `echo` is on and the shell prompt behaves normally.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux
git commit -m "feat(neotmux): main loop, pty wiring and terminal lifecycle"
```

---

### Task 8: Developer entry points

**Files:**
- Create: `Makefile`
- Modify: `README.md`

**Interfaces:**
- Consumes: the workspace from Tasks 1–7
- Produces: `make check`, `make test`, `make run`, `make fmt`

- [ ] **Step 1: Write the Makefile**

```make
.PHONY: all check fmt lint test run

all: check

check: fmt lint test

fmt:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

run:
	cargo run -p neotmux
```

- [ ] **Step 2: Run it**

Run: `make check`
Expected: fmt, clippy and tests all pass. Fix anything clippy reports rather than allowing it.

- [ ] **Step 3: Update the README status section**

Replace the `## Status` section of `README.md` with:

```markdown
## Status

M0a (walking skeleton) works: `neotmux` runs a shell full-screen and exits cleanly.
There is no plugin system yet — that is M1a. See `docs/superpowers/specs/` for the design
and `docs/superpowers/plans/` for the implementation plans.

```sh
make run     # run it
make check   # fmt + clippy + tests
```
```

- [ ] **Step 4: Commit**

```bash
git add Makefile README.md
git commit -m "chore: make targets and README status"
```

---

## Definition of done for M0a

Taken from the spec's exit criteria, all machine-checkable:

- [ ] A PTY harness writes `printf 'X'`; after render, cell (0,0) is `X` — `shell_output.rs`
- [ ] The child exits 0 and the process exits 0 — `terminal_restore.rs`
- [ ] The alternate screen is left on clean exit — `terminal_restore.rs`
- [ ] The alternate screen is left when the host panics — `terminal_restore.rs`
- [ ] `make check` is clean

Deferred to later milestones by design: splits, layout, detach, scrollback UI, copy-mode, Lua.

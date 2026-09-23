# Crate Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the one forbidden edge in the workspace (`neotmux-lua` → `neotmux-term`) and put the pane tree where the design says it lives, so the dependency graph and the code's placement both match §2 and §2.1 of the design spec.

**Architecture:** Four moves. The pane-read vocabulary, which exists twice today (once for the process in `neotmux-term`, once for the wire in `neotmux-proto`), becomes one vocabulary in `neotmux-proto`, trait included. The clipboard stops being a write to `stdout` from inside the Lua crate and becomes a trait the host implements. The column-width rule moves to the crate that owns the grid and its two hand-rolled copies start calling it. `MIN_GRID` becomes a parameter of `PaneLayout` rather than a constant it imports, which is the last thing standing between `panes.rs`/`windows.rs` and `neotmux-core`.

**Tech Stack:** Rust 2024, one workspace, no new dependencies anywhere. `unicode-width` moves from three direct dependents to one.

**Spec:** `docs/superpowers/specs/2026-09-23-crate-boundaries-design.md`

## Global Constraints

- **No behaviour changes.** Nothing a user can see moves. The one case this was expected to correct (§6) turns out to be unreachable, so even that does not move.
- **No Lua ABI changes.** `neotmux.width`, `neotmux.clipboard.set`, `neotmux.pane.*` keep their names, arguments, return values and the meaning of those returns. `neotmux.clipboard.set` in particular keeps answering two values, and its `true` keeps meaning "the sequence was written to the client's terminal and flushed".
- **No wire changes.** `PROTOCOL_VERSION` stays at 3. No message gains, loses or reshapes a field on the wire. Task 3 proves this with a test rather than asserting it.
- **Dependency edges point downward only.** `neotmux-proto` ← everything; `neotmux-core`, `neotmux-term`, `neotmux-lua` are siblings that must not depend on each other; `neotmux` depends on all four.
- **`neotmux-core` must stay testable with no terminal and no Lua.** Spec §2 calls this "the check on whether the boundaries are real".
- **Gate for every task:** `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings` clean, plus the task's own test command.
- **Known-flaky test, not this plan's:** `crates/neotmux/tests/fixtures.rs:444` (`spinner_is_dropped_while_the_rest_of_the_program_carries_on`, both callers) fails roughly one run in six on a loaded machine and does so at `a4fa182` too. A failure there is not a regression from this plan; re-run it before investigating.

---

### Task 1: One column rule, in `neotmux-proto`

`neotmux-term::width` is called from exactly one production site — `neotmux.width` in the Lua crate — while two other places measure columns with their own copy of the rule. All three agree; the point of this task is that they agree structurally rather than by luck, and that `unicode-width` stops being a direct dependency of three crates.

**Files:**
- Create: `crates/neotmux-proto/src/width.rs`
- Modify: `crates/neotmux-proto/src/lib.rs` (add `pub mod width;`)
- Modify: `crates/neotmux-proto/Cargo.toml` (add `unicode-width`)
- Delete: `crates/neotmux-term/src/width.rs`
- Modify: `crates/neotmux-term/src/lib.rs` (drop `pub mod width;`)
- Modify: `crates/neotmux-term/src/lines.rs:170` (its test calls `crate::width::of_str`)
- Modify: `crates/neotmux-term/Cargo.toml` (drop `unicode-width`)
- Modify: `crates/neotmux-lua/src/api/mod.rs:306` (`neotmux_term::width::of_str` → `neotmux_proto::width::of_str`)
- Modify: `crates/neotmux-lua/src/float.rs:5,134` (`clip_text` calls the shared rule)
- Modify: `crates/neotmux-lua/Cargo.toml` (drop `unicode-width`)
- Modify: `crates/neotmux/src/overlay.rs:5,94` (`draw_text` calls the shared rule)
- Modify: `crates/neotmux/Cargo.toml` (drop `unicode-width`)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `neotmux_proto::width::of_char(c: char) -> usize` and `neotmux_proto::width::of_str(text: &str) -> usize`. Task 3 does not use them; no later task depends on this one.

- [ ] **Step 1: Write the failing test**

Create `crates/neotmux-proto/src/width.rs` with only its tests for now, so the file exists and the test names the function that does not:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The arm `of_char` keeps for `vt100`'s sake — an unmeasurable character at U+0100 or
    /// above is given one column — cannot be reached with `unicode-width` 0.2: the 65 code
    /// points it declines to measure are the C0 and C1 controls and DEL, all below U+0100.
    ///
    /// Kept as a test rather than deleted as dead code, because if a later `unicode-width`
    /// does decline one higher up, this fails here rather than silently splitting the rule
    /// between `of_char` and the two callers that wrote `unwrap_or(0)`.
    #[test]
    fn nothing_at_or_above_u0100_is_unmeasurable() {
        use unicode_width::UnicodeWidthChar as _;
        let unmeasurable: Vec<u32> = (0u32..=0x10FFFF)
            .filter_map(char::from_u32)
            .filter(|c| c.width().is_none())
            .map(u32::from)
            .collect();
        assert_eq!(unmeasurable.len(), 65, "{unmeasurable:?}");
        assert!(
            unmeasurable.iter().all(|&c| c < 0x100),
            "the arm is reachable now: {:?}",
            unmeasurable
                .iter()
                .filter(|&&c| c >= 0x100)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_wide_character_is_two_columns_and_a_combining_mark_is_none() {
        assert_eq!(of_char('あ'), 2);
        assert_eq!(of_char('a'), 1);
        assert_eq!(of_char('\u{0301}'), 0); // combining acute
        assert_eq!(of_char('\u{1b}'), 0); // ESC, a control below U+0100
        assert_eq!(of_str("aあ"), 3);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Add `pub mod width;` to `crates/neotmux-proto/src/lib.rs` and `unicode-width.workspace = true` to `crates/neotmux-proto/Cargo.toml` first, or the failure is the wrong one.

Run: `cargo test -p neotmux-proto --lib width::`
Expected: FAIL to compile — `cannot find function of_char in this scope`.

- [ ] **Step 3: Write the implementation**

Put this above the `mod tests` block in `crates/neotmux-proto/src/width.rs`. It is `crates/neotmux-term/src/width.rs` moved verbatim, with one sentence added to the doc and one to the unreachable arm:

```rust
//! How many columns a string takes on a pane's grid.
//!
//! The rule is `vt100` 0.16.2's own, taken from `Screen::text`, because the grid a pane is
//! drawn from is `vt100`'s: a character's width decides whether the cell it lands in is
//! marked wide, and the renderer skips the continuation cell of a wide one. Measuring any
//! other way would let a plugin's columns and the renderer's columns disagree, which is the
//! whole reason this is in the host rather than in Lua — and why it is here, beside the grid
//! it describes, rather than in the crate that happens to own the emulator.
//!
//! Two consequences worth naming. A zero-width character — a combining mark — adds nothing,
//! because `vt100` appends it to the cell before it rather than giving it one. A control
//! character below U+0100 adds nothing either, because `vt100` refuses to draw one at all;
//! a tab never reaches a cell, it moves the cursor, so it has no width here.

use unicode_width::UnicodeWidthChar as _;

/// How many columns `c` occupies in a pane.
pub fn of_char(c: char) -> usize {
    match c.width() {
        Some(width) => width,
        // `Screen::text`: an unmeasurable character below U+0100 is a control character and
        // is not drawn; anything else it cannot measure is given one column. The second arm
        // cannot be reached with `unicode-width` 0.2 — everything it declines to measure is
        // a control below U+0100 — and is kept because this mirrors `vt100`'s own match, and
        // a mirror that drops a case stops being one. `nothing_at_or_above_u0100_is_unmeasurable`
        // is what says so.
        None if u32::from(c) < 256 => 0,
        None => 1,
    }
}

/// How many columns `text` occupies in a pane.
pub fn of_str(text: &str) -> usize {
    text.chars().map(of_char).sum()
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p neotmux-proto --lib width::`
Expected: PASS, 2 tests.

- [ ] **Step 5: Delete the old copy and point `neotmux-term` at the new one**

Delete `crates/neotmux-term/src/width.rs`. Remove `pub mod width;` from `crates/neotmux-term/src/lib.rs`. Remove `unicode-width.workspace = true` from `crates/neotmux-term/Cargo.toml`. In `crates/neotmux-term/src/lines.rs:170`, change `crate::width::of_str(&line)` to `neotmux_proto::width::of_str(&line)`.

Run: `cargo test -p neotmux-term`
Expected: PASS, no change in count.

- [ ] **Step 6: Point the Lua crate's two sites at it**

`crates/neotmux-lua/src/api/mod.rs:306`: `neotmux_term::width::of_str(&text)` → `neotmux_proto::width::of_str(&text)`.

`crates/neotmux-lua/src/float.rs`: drop `use unicode_width::UnicodeWidthChar;` from line 5, and at line 134 replace

```rust
        let width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
```

with

```rust
        let width = neotmux_proto::width::of_char(ch) as u16;
```

Update `clip_text`'s doc line so it names the shared rule rather than a twin:

```rust
/// Same truncation rule as the renderer's `overlay.rs::draw_text`, because both call
/// `neotmux_proto::width`: a zero-width (combining) char rides the previous cell rather than
/// being measured, and a wide char that would straddle the cap drops as a whole rather than
/// being half-drawn.
```

Remove `unicode-width.workspace = true` from `crates/neotmux-lua/Cargo.toml`.

Run: `cargo test -p neotmux-lua`
Expected: PASS, no change in count. `crates/neotmux-lua/tests/width.rs` and `drawing.rs` are the ones that would notice.

- [ ] **Step 7: Point the renderer at it**

`crates/neotmux/src/overlay.rs`: drop `use unicode_width::UnicodeWidthChar;` from line 5, and at line 94 replace

```rust
        let width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
```

with

```rust
        let width = neotmux_proto::width::of_char(ch) as u16;
```

Remove `unicode-width.workspace = true` from `crates/neotmux/Cargo.toml` (the `[dependencies]` entry; there is no dev one).

Run: `cargo test -p neotmux --test fixtures`
Expected: PASS, 97 tests. (See the Global Constraints note on the known flake.)

- [ ] **Step 8: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast`
Expected: clean, and every suite green but the known flake.

- [ ] **Step 9: Commit**

```bash
git add crates/neotmux-proto crates/neotmux-term crates/neotmux-lua crates/neotmux
git commit -m "one column rule, in the crate that owns the grid"
```

---

### Task 2: The clipboard is a contract the host satisfies

**Files:**
- Create: `crates/neotmux-lua/src/clipboard.rs`
- Modify: `crates/neotmux-lua/src/lib.rs` (declare the module; add the field to `LuaSubsystem`; `build` and `build_named` signatures)
- Modify: `crates/neotmux-lua/src/api/mod.rs` (add `clipboard` to `Shared`; pass it to `clipboard::install`)
- Modify: `crates/neotmux-lua/src/api/clipboard.rs` (take the contract instead of `io::stdout()`)
- Modify: `crates/neotmux/src/main.rs:343` (the client implements it)
- Test: `crates/neotmux-lua/tests/clipboard.rs` (one test added)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `neotmux_lua::clipboard::Clipboard` — `fn set(&self, text: &str) -> std::io::Result<()>`
  - `neotmux_lua::clipboard::Absent` — a unit struct whose `set` returns `Err`, used by `LuaSubsystem::build`
  - `LuaSubsystem::build_named(session_name: impl Into<String>, layers: Layers, panes: Rc<dyn PaneRead>, clipboard: Rc<dyn Clipboard>) -> anyhow::Result<Self>`
  - `LuaSubsystem::build(layers: Layers, panes: Rc<dyn PaneRead>) -> anyhow::Result<Self>` — unchanged signature, supplies `Absent`

Task 6 relies on this task having removed the last non-read use of `neotmux-term` from the Lua crate's `[dependencies]`.

- [ ] **Step 1: Write the failing test**

Append to `crates/neotmux-lua/tests/clipboard.rs`:

```rust
/// The host's implementation is what a write reaches, and what it answers is what the plugin
/// is told. The Lua crate does not own a terminal to write to.
#[test]
fn a_write_reaches_the_clipboard_the_host_gave() {
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Recording(RefCell<Vec<String>>);
    impl neotmux_lua::clipboard::Clipboard for Recording {
        fn set(&self, text: &str) -> std::io::Result<()> {
            self.0.borrow_mut().push(text.to_string());
            Ok(())
        }
    }

    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "neotmux.on_tick(function()\n\
         local ok, err = neotmux.clipboard.set('yanked')\n\
         neotmux.log.info(tostring(ok) .. ' ' .. tostring(err))\n\
         end)\n",
    )
    .unwrap();

    let recording = Rc::new(Recording(RefCell::new(Vec::new())));
    let sub = LuaSubsystem::build_named(
        "default",
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
        recording.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());

    assert_eq!(ticked(&sub), vec!["true nil"]);
    assert_eq!(*recording.0.borrow(), vec!["yanked".to_string()]);
}

/// A subsystem built without one says so rather than claiming a write it did not make. This
/// is what `LuaSubsystem::build` gives, and `build` is for tests and the headless loop.
#[test]
fn without_a_clipboard_the_answer_is_false_and_a_reason() {
    let (_dir, sub) = built(
        "local ok, err = neotmux.clipboard.set('yanked')\n\
         neotmux.log.info(tostring(ok) .. ' ' .. tostring(err))",
    );
    let line = ticked(&sub).remove(0);
    assert_eq!(line, "false this host has no clipboard");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p neotmux-lua --test clipboard`
Expected: FAIL to compile — `could not find clipboard in neotmux_lua`, and `build_named` takes 3 arguments.

- [ ] **Step 3: Write the contract**

Create `crates/neotmux-lua/src/clipboard.rs`:

```rust
//! What the host owes a plugin that wants to put something on the clipboard.
//!
//! The Lua layer has no terminal. It runs beside one — M3 puts it in the client — but the
//! handle, and the escape sequence that goes down it, are the host's, the same way a pane's
//! text is the host's and reaches Lua through [`neotmux_proto::read::PaneRead`]. A contract
//! rather than a write is what keeps this crate from knowing either.
//!
//! Declared here rather than in `neotmux-proto`, because the only implementor is the client
//! binary, which already depends on this crate: a contract belongs in the lowest crate that
//! both its consumer and every one of its implementors can see, and for this one that is
//! here. Nothing puts it on the wire; the client writes to its own terminal.

/// Somewhere to put text a plugin has yanked.
pub trait Clipboard {
    /// Hands `text` over. `Ok` means the host did what it does — for the client that is
    /// writing the sequence to its terminal and flushing it — and never that a clipboard now
    /// holds the text, which nothing in this direction can know.
    fn set(&self, text: &str) -> std::io::Result<()>;
}

/// The one a subsystem built with [`crate::LuaSubsystem::build`] gets: a host that has no
/// clipboard at all, which answers so rather than claiming a write it did not make.
pub struct Absent;

impl Clipboard for Absent {
    fn set(&self, _text: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("this host has no clipboard"))
    }
}
```

- [ ] **Step 4: Thread it through the subsystem**

In `crates/neotmux-lua/src/lib.rs`: add `pub mod clipboard;` beside the other module declarations, and `use clipboard::Clipboard;` beside the other imports.

Change the two constructors:

```rust
    pub fn build(layers: Layers, panes: Rc<dyn PaneRead>) -> anyhow::Result<Self> {
        Self::build_named("default", layers, panes, Rc::new(clipboard::Absent))
    }

    /// The named half of `build`, which passes the default: `neotmux.session_name()` is
    /// the one that answers whichever of the two the client was started with.
    pub fn build_named(
        session_name: impl Into<String>,
        layers: Layers,
        panes: Rc<dyn PaneRead>,
        clipboard: Rc<dyn Clipboard>,
    ) -> anyhow::Result<Self> {
```

and pass it into the `api::Shared` literal, beside `panes`:

```rust
                panes,
                clipboard,
```

- [ ] **Step 5: Give the API the contract instead of stdout**

In `crates/neotmux-lua/src/api/mod.rs`, add to `Shared` beside `panes`:

```rust
    pub(crate) clipboard: Rc<dyn crate::clipboard::Clipboard>,
```

destructure it in `install` beside `panes`, and change the call at line 274:

```rust
        let clipboard = clipboard::install(lua, clipboard)?;
```

In `crates/neotmux-lua/src/api/clipboard.rs`, replace `write_to` and `install` with:

```rust
use std::rc::Rc;

use mlua::{Lua, Table};

use crate::clipboard::Clipboard;

pub(crate) fn install(lua: &Lua, clipboard: Rc<dyn Clipboard>) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    {
        // The host's, because the terminal is the host's: this crate has none to write to.
        // Called from a tick, between frames, so whatever the host does with it cannot land
        // inside a frame's own output.
        let set = lua.create_function(move |_, text: String| {
            Ok(match clipboard.set(&text) {
                Ok(()) => (true, None),
                Err(err) => (false, Some(err.to_string())),
            })
        })?;
        table.set("set", set)?;
    }

    Ok(table)
}
```

Keep the file's existing module doc — every sentence of it is still true, and it is the only place the meaning of `true` is written down. Delete its two `mod tests` cases: they tested `write_to`, which is now the host's, and `a_write_reaches_the_clipboard_the_host_gave` in `tests/clipboard.rs` is what covers the path.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p neotmux-lua --test clipboard`
Expected: PASS, 4 tests.

- [ ] **Step 7: Give the client its implementation**

In `crates/neotmux/src/main.rs`, above the call site at line 343:

```rust
/// The client's clipboard: the OSC 52 sequence, written to the terminal the renderer draws
/// on and flushed. Flushing is not optional — the sequence carries no newline, so a
/// line-buffered terminal would otherwise hold it until something else came along.
struct TerminalClipboard;

impl neotmux_lua::clipboard::Clipboard for TerminalClipboard {
    fn set(&self, text: &str) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        out.write_all(&neotmux_term::osc52::set(text))?;
        out.flush()
    }
}
```

and change the call:

```rust
    let lua = LuaSubsystem::build_named(
        name,
        layers,
        panes.clone(),
        std::rc::Rc::new(TerminalClipboard),
    )?;
```

`crates/neotmux/Cargo.toml` already depends on `neotmux-term`; nothing to add.

- [ ] **Step 8: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast`
Expected: clean, every suite green but the known flake.

- [ ] **Step 9: Commit**

```bash
git add crates/neotmux-lua crates/neotmux
git commit -m "the clipboard is something the host has, not something the Lua layer writes"
```

---

### Task 3: One pane-read vocabulary, in `neotmux-proto`

**Files:**
- Create: `crates/neotmux-proto/src/read.rs`
- Modify: `crates/neotmux-proto/src/lib.rs` (declare and re-export)
- Modify: `crates/neotmux-proto/src/msg.rs:146-205` (`Read::Content`, `Read::History` take a `Want`; `Lines` and `Extent` move out to `read.rs` and are re-exported from `msg` so existing paths keep working)
- Modify: `crates/neotmux-proto/src/wire.rs:997-1025, 1373-1385, 1981-1985` (encode/decode a `Want`; the test fixture)
- Delete: `crates/neotmux-term/src/read.rs`
- Modify: `crates/neotmux-term/src/lib.rs` (drop `pub mod read;`)
- Modify: `crates/neotmux-term/src/lines.rs` (move `Trailing` out; keep `row`)
- Modify: `crates/neotmux-term/src/terminals.rs:182-211` and its inherent methods (take and return the proto types)
- Modify: `crates/neotmux-term/src/scrollback.rs`, `wrap.rs`, `state.rs` wherever `Text`/`History`/`Want`/`Trailing` are named
- Modify: `crates/neotmux-lua/src/lib.rs:27`, `src/api/mod.rs:18`, `src/api/pane.rs:7-8`
- Modify: `crates/neotmux/src/reads.rs` (the conversion goes)
- Modify: `crates/neotmux-lua/tests/copy.rs:156`
- Test: `crates/neotmux-proto/src/wire.rs` (`mod tests`), one test added

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces, all in `neotmux_proto::read` and re-exported from the crate root:
  - `Trailing` — `Trim` (default) | `Pad`
  - `Want { trailing: Trailing, wrapped: bool }`
  - `Lines { lines: Vec<String>, wrapped: Option<Vec<bool>>, revision: u64 }`
  - `Extent { len: u64, dropped: u64 }`
  - `PaneRead` — the seven methods below

Task 6 relies on this task having removed the last `[dependencies]` use of `neotmux-term` from the Lua crate.

- [ ] **Step 1: Write the failing test**

Add to `crates/neotmux-proto/src/wire.rs`'s `mod tests`:

```rust
/// The bytes did not move when `pad: bool, wrapped: bool` became `want: Want`.
///
/// A `Trailing` has two variants, so it encodes as the one byte the `pad` bool encoded as,
/// and `Want` is that byte followed by the `wrapped` byte — the order the two were already
/// written in. The fixture is the encoding as it was before the change, recorded by hand, so
/// that this is a claim about bytes and not about the code that produces them: if it fails,
/// `PROTOCOL_VERSION` moves and the design's §4 is wrong.
#[test]
fn a_content_read_encodes_the_bytes_it_encoded_before_want() {
    let read = Read::Content {
        pane: PaneId::from(7),
        last: Some(3),
        want: Want {
            trailing: Trailing::Pad,
            wrapped: false,
        },
    };
    let encoded = encode_to_server(&ToServer::Read(read.clone()));

    // pad = true, wrapped = false — the last two bytes of the message, unchanged.
    assert_eq!(
        &encoded[encoded.len() - 2..],
        &[1, 0],
        "the two option bytes moved: {encoded:?}"
    );
    assert_eq!(
        decode_to_server(&encoded).expect("decodes"),
        ToServer::Read(read)
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p neotmux-proto --lib wire::tests::a_content_read_encodes`
Expected: FAIL to compile — `Read::Content` has no field `want`, and `Want`/`Trailing` are not in scope.

- [ ] **Step 3: Write the vocabulary**

Create `crates/neotmux-proto/src/read.rs`. `Trailing`, `Want` and `PaneRead` are `crates/neotmux-term/src/read.rs` and the `Trailing` half of `lines.rs`, moved, with `usize` become `u64` and `Text`/`History` become the wire's `Lines`/`Extent`:

```rust
//! Asking a pane for its text, and what comes back.
//!
//! One vocabulary, not two. These are the types the Lua layer names, the types
//! [`crate::msg::Read`] and [`crate::msg::Reading`] carry across the socket, and the types
//! the local terminals answer in — which is §2.1's rule about the frame ("one definition
//! each, no conversion layer") applied to the reads, where it had not been.
//!
//! [`PaneRead`] is here rather than beside either implementation because a contract belongs
//! in the lowest crate that both its consumer and every one of its implementors can see. The
//! consumer is the Lua layer; the implementors are the session's own terminals and, in the
//! client, a stand-in that asks the socket. Nothing below this crate is common to all three.

use crate::PaneId;

/// What to do with the blank rest of a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trailing {
    /// Cut it, so an untouched row is `""`. What reading a line of text wants.
    #[default]
    Trim,
    /// Keep it, so every row is exactly as wide as the pane. What a rectangle wants: the
    /// interior spacing of a short line is where its columns are.
    Pad,
}

/// What to include in a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// What to do with the blank rest of each row.
    pub trailing: Trailing,
    /// Whether to also report, per row, if its text runs on into the next one. Off by
    /// default: a reader that does not need it should not pay for the pass.
    pub wrapped: bool,
}

/// One read of a pane's text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    pub lines: Vec<String>,
    /// Whether each line runs on into the next, the same length as `lines` — or `None`
    /// where it was not asked for. `None` is "not asked", never "none of them wrap".
    pub wrapped: Option<Vec<bool>>,
    /// The revision the read was taken at.
    pub revision: u64,
}

/// Where a pane's history begins, and how long it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extent {
    /// How many lines the pane holds above its visible screen.
    pub len: u64,
    /// How many lines have fallen out of the front of it since the pane opened, so history
    /// line `i` is the `dropped + i`th line the pane ever scrolled away.
    pub dropped: u64,
}

/// Reading a pane's text, from wherever the pane's terminal happens to be.
///
/// `neotmux-term`'s `PaneTerminals` is the local answer, and the one the session holds. The
/// Lua layer runs in the process with the screen rather than the one with the panes, so its
/// reads are answered across a socket instead — same questions, a different distance, and
/// since M4 the same types either way.
pub trait PaneRead {
    fn revision(&self, pane: PaneId) -> Option<u64>;
    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines>;
    fn history_extent(&self, pane: PaneId) -> Option<Extent>;
    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines>;
    fn freeze(&self, pane: PaneId) -> bool;
    fn unfreeze(&self, pane: PaneId) -> bool;
    fn is_frozen(&self, pane: PaneId) -> Option<bool>;
}
```

In `crates/neotmux-proto/src/lib.rs` add `pub mod read;` and to the re-exports:

```rust
pub use read::{Extent, Lines, PaneRead, Trailing, Want};
```

In `crates/neotmux-proto/src/msg.rs`, delete the `Lines` and `Extent` definitions (lines 179-191) and re-export them where they were, so every existing `msg::Lines` path keeps working:

```rust
pub use crate::read::{Extent, Lines, Want};
```

and change the two variants:

```rust
    Content {
        pane: PaneId,
        last: Option<u64>,
        want: Want,
    },
    …
    History {
        pane: PaneId,
        from: u64,
        count: u64,
        want: Want,
    },
```

- [ ] **Step 4: Teach the codec the `Want`**

In `crates/neotmux-proto/src/wire.rs`, the two encode arms at 997-1025 become this — every
write is in the order it was, and the two `bool`s are the two `bool`s, which is the whole of
§4:

```rust
            Read::Content {
                pane,
                last,
                want,
            } => {
                self.u8(READ_CONTENT);
                self.u32(pane.get());
                self.opt_u64(last);
                self.bool(matches!(want.trailing, Trailing::Pad));
                self.bool(want.wrapped);
            }
            …
            Read::History {
                pane,
                from,
                count,
                want,
            } => {
                self.u8(READ_HISTORY);
                self.u32(pane.get());
                self.u64(from);
                self.u64(count);
                self.bool(matches!(want.trailing, Trailing::Pad));
                self.bool(want.wrapped);
            }
```

The `Read::Revision`, `Read::HistoryExtent`, `Read::Freeze`, `Read::Unfreeze` and
`Read::IsFrozen` arms between and around them are untouched. The decode arms at 1373-1385:

```rust
            READ_CONTENT => Read::Content {
                pane: self.pane()?,
                last: self.opt_u64()?,
                want: Want {
                    trailing: if self.bool()? {
                        Trailing::Pad
                    } else {
                        Trailing::Trim
                    },
                    wrapped: self.bool()?,
                },
            },
```

and the same for `READ_HISTORY`. Keep whatever the existing arms call to read `pane` and the numbers; only the last two reads change shape.

The existing fixture at 1981-1985 becomes:

```rust
            Read::Content {
                pane: pane(1),
                last: None,
                want: Want {
                    trailing: Trailing::Pad,
                    wrapped: false,
                },
            },
```

Add `use crate::read::{Trailing, Want};` to `wire.rs`'s imports if the re-export does not already bring them into scope.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p neotmux-proto`
Expected: PASS. The new test and every existing wire round-trip.

- [ ] **Step 6: Point `neotmux-term` at the vocabulary**

Delete `crates/neotmux-term/src/read.rs` and drop `pub mod read;` from `crates/neotmux-term/src/lib.rs`. Delete the `Trailing` enum from `crates/neotmux-term/src/lines.rs` and add `use neotmux_proto::read::Trailing;` there instead — `row` keeps taking it and keeps its `vt100::Screen` argument, which is why `lines.rs` stays in this crate.

Four files name those types and have to be repointed: `terminals.rs`, `scrollback.rs`,
`wrap.rs` and `state.rs` (`query.rs` does not). In each, `crate::read::{Want, Text, History}`
becomes `neotmux_proto::read::{Want, Lines, Extent}` — take the names each file actually
uses, not all three. The inherent methods on `PaneTerminals` keep `usize` where they index, and the `impl PaneRead for PaneTerminals` block at 182-211 is where the cast happens — this crate pays for its own indexing type:

```rust
impl PaneRead for PaneTerminals {
    fn revision(&self, pane: PaneId) -> Option<u64> {
        PaneTerminals::revision(self, pane)
    }

    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines> {
        PaneTerminals::content(self, pane, last.map(|n| n as usize), want)
    }

    fn history_extent(&self, pane: PaneId) -> Option<Extent> {
        PaneTerminals::history_extent(self, pane)
    }

    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines> {
        PaneTerminals::history(self, pane, from as usize, count as usize, want)
    }

    fn freeze(&self, pane: PaneId) -> bool {
        PaneTerminals::freeze(self, pane)
    }

    fn unfreeze(&self, pane: PaneId) -> bool {
        PaneTerminals::unfreeze(self, pane)
    }

    fn is_frozen(&self, pane: PaneId) -> Option<bool> {
        PaneTerminals::is_frozen(self, pane)
    }
}
```

The inherent `content`/`history` now return `Lines` and `history_extent` returns `Extent`; where they built a `Text`/`History` they build the same fields with `as u64` on the two counts.

Run: `cargo test -p neotmux-term`
Expected: PASS, no change in count.

- [ ] **Step 7: Point the Lua crate at the vocabulary**

`crates/neotmux-lua/src/lib.rs:27`: `use neotmux_term::read::PaneRead;` → `use neotmux_proto::read::PaneRead;`
`crates/neotmux-lua/src/api/mod.rs:18`: the same.
`crates/neotmux-lua/src/api/pane.rs:7-8`: `use neotmux_term::lines::Trailing;` and `use neotmux_term::read::{PaneRead, Text, Want};` become `use neotmux_proto::read::{Lines, PaneRead, Trailing, Want};`, and the body's `Text` becomes `Lines`. Where it passed a `usize` to `content`/`history` it passes the `u64` it already had from Lua, so the `as usize` casts there go.
`crates/neotmux-lua/tests/copy.rs:156`: `neotmux_term::read::Want::default()` → `neotmux_proto::read::Want::default()`.

Run: `cargo test -p neotmux-lua`
Expected: PASS, no change in count.

- [ ] **Step 8: Delete the conversion**

`crates/neotmux/src/reads.rs`: the imports at 24-25 become `use neotmux_proto::read::{Extent, Lines, PaneRead, Want};`, the `fn pad` helper goes, and the two halves become direct:

```rust
impl PaneRead for RemotePanes {
    fn revision(&self, pane: PaneId) -> Option<u64> {
        self.revisions.borrow().get(&pane).copied()
    }

    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines> {
        self.text(pane, Read::Content { pane, last, want })
    }

    fn history_extent(&self, pane: PaneId) -> Option<Extent> {
        match self.ask(Read::HistoryExtent { pane })? {
            Reading::Extent(extent) => extent,
            _ => None,
        }
    }

    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines> {
        self.text(
            pane,
            Read::History {
                pane,
                from,
                count,
                want,
            },
        )
    }

    fn freeze(&self, pane: PaneId) -> bool {
        self.did(Read::Freeze { pane })
    }

    fn unfreeze(&self, pane: PaneId) -> bool {
        self.did(Read::Unfreeze { pane })
    }

    fn is_frozen(&self, pane: PaneId) -> Option<bool> {
        match self.ask(Read::IsFrozen { pane })? {
            Reading::Frozen(frozen) => frozen,
            _ => None,
        }
    }
}

/// The other side of [`RemotePanes`]: one read, answered out of the session's own terminals.
///
/// One line per question, because there is one vocabulary now: what the socket asks in is
/// what the terminals answer in.
pub fn answer(panes: &dyn PaneRead, read: Read) -> Reading {
    match read {
        Read::Revision { pane } => Reading::Revision(panes.revision(pane)),
        Read::Content { pane, last, want } => Reading::Text(panes.content(pane, last, want)),
        Read::HistoryExtent { pane } => Reading::Extent(panes.history_extent(pane)),
        Read::History {
            pane,
            from,
            count,
            want,
        } => Reading::Text(panes.history(pane, from, count, want)),
        Read::Freeze { pane } => Reading::Did(panes.freeze(pane)),
        Read::Unfreeze { pane } => Reading::Did(panes.unfreeze(pane)),
        Read::IsFrozen { pane } => Reading::Frozen(panes.is_frozen(pane)),
    }
}
```

`RemotePanes::text` returns `Option<Lines>` directly now — where it unwrapped a `Reading::Text(Option<Lines>)` into a `Text`, it returns the `Option<Lines>` as it is.

- [ ] **Step 9: Run the whole workspace**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast`
Expected: clean, every suite green but the known flake. `crates/neotmux/tests/socket.rs` (25) and `crates/neotmux-lua/tests/{content,copy,scrollback,freeze,watch}.rs` are the ones that would notice a mistake here.

- [ ] **Step 10: Commit**

```bash
git add crates
git commit -m "one vocabulary for a pane read: the question the socket asks is the one the terminals answer"
```

---

### Task 4: The minimum is the caller's

**Files:**
- Modify: `crates/neotmux/src/panes.rs:1-3, 36-54, 57-72, 107-125` and its `mod tests`
- Modify: `crates/neotmux/src/windows.rs` (wherever it constructs a `PaneLayout`)
- Modify: `crates/neotmux/src/session.rs` (the one place a `PaneLayout` or `Surface` is first made)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `PaneLayout::new(full: Size, reserved: Reserved, gutter: u16, first: PaneId, minimum: Size) -> Self`, and a `minimum: Size` field carried on `PaneLayout` and on `windows::Surface`. Task 5 moves both files and depends on this signature.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux/src/panes.rs`'s `mod tests`, replace the bare `MIN_GRID` uses with an explicit minimum and add the case that says the number is the caller's and not this file's:

```rust
    /// The minimum is the caller's (design §"Rounding and minimum sizes"), so a caller that
    /// says something other than `MIN_GRID` gets what it said. Nothing in this file knows
    /// what `vt100` can survive.
    #[test]
    fn the_minimum_a_caller_gives_is_the_minimum_that_is_used() {
        let big = Size { rows: 10, cols: 10 };
        let reserved = bottom(1);
        let layout = PaneLayout::new(Size { rows: 10, cols: 80 }, reserved, 0, pane(1), big);
        assert_eq!(
            layout.applied_bands(),
            Reserved::default(),
            "ten rows cannot hold a band and a ten-row minimum"
        );

        let small = Size { rows: 2, cols: 2 };
        let layout = PaneLayout::new(Size { rows: 10, cols: 80 }, reserved, 0, pane(1), small);
        assert_eq!(
            layout.applied_bands(),
            reserved,
            "the same terminal holds the band when the caller asks for less"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p neotmux --lib panes::`
Expected: FAIL to compile — `PaneLayout::new` takes 4 arguments.

- [ ] **Step 3: Make the minimum a field**

In `crates/neotmux/src/panes.rs`: drop `use neotmux_term::state::MIN_GRID;`. Add to the struct, with the reason:

```rust
    /// The smallest a pane may be, as the caller gave it. Not a constant here: the design
    /// puts it with the terminal ("the smallest grid `vt100` 0.16.2 survives, not a property
    /// of layout") and passes it in, which is also what keeps this file free of a terminal.
    minimum: Size,
```

`applied` and `showable` become methods, or take the minimum as an argument:

```rust
fn applied(full: Size, reserved: Reserved, minimum: Size) -> Reserved {
    let mut applied = Reserved::default();
    if full.rows >= reserved.rows().saturating_add(minimum.rows) {
        applied.add(Edge::Top, reserved.top);
        applied.add(Edge::Bottom, reserved.bottom);
    }
    if full.cols >= reserved.cols().saturating_add(minimum.cols) {
        applied.add(Edge::Left, reserved.left);
        applied.add(Edge::Right, reserved.right);
    }
    applied
}

fn showable(rect: PRect, minimum: Size) -> bool {
    rect.w >= minimum.cols && rect.h >= minimum.rows
}
```

`new` takes and stores it; `resurface`, `split`, `resize` and the `showable` iterator pass `self.minimum` where they passed `MIN_GRID`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux --lib panes::`
Expected: PASS, the existing cases plus the new one.

- [ ] **Step 5: Thread the minimum from the binary**

`crates/neotmux/src/windows.rs:10`: `Surface` gains `pub minimum: Size`, and passes
`self.minimum` wherever it builds a `PaneLayout`. Its own test fixtures at 208-211 and 310
gain the field too — `Size { rows: 2, cols: 2 }` there, the value `MIN_GRID` holds, so the
tests keep asserting what they asserted.

`crates/neotmux/src/session.rs:85` and `:809` are the two places a `Surface` is built. Both
pass `minimum: neotmux_term::state::MIN_GRID`. That is where the constant belongs: the binary
owns the map from a pane to its `vt100` parser, so the binary is what knows what `vt100`
survives.

- [ ] **Step 6: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast`
Expected: clean, every suite green but the known flake.

- [ ] **Step 7: Commit**

```bash
git add crates/neotmux
git commit -m "the smallest a pane may be is the caller's number, not the layout's"
```

---

### Task 5: `PaneLayout` and the windows over it move to `neotmux-core`

**Files:**
- Create: `crates/neotmux-core/src/panes.rs` (from `crates/neotmux/src/panes.rs`)
- Create: `crates/neotmux-core/src/windows.rs` (from `crates/neotmux/src/windows.rs`)
- Modify: `crates/neotmux-core/src/lib.rs` (declare both)
- Delete: `crates/neotmux/src/panes.rs`, `crates/neotmux/src/windows.rs`
- Modify: `crates/neotmux/src/lib.rs` (drop `pub mod panes;` and `mod windows;`)
- Modify: `crates/neotmux/src/session.rs`, `crates/neotmux/src/shells.rs` (the import paths)

**Interfaces:**
- Consumes: `PaneLayout::new(full, reserved, gutter, first, minimum)` and `windows::Surface` with its `minimum` field, both from Task 4.
- Produces: `neotmux_core::panes::PaneLayout` and `neotmux_core::windows::{Surface, …}` — the same items under new paths. No signature changes.

- [ ] **Step 1: Move the files**

```bash
git mv crates/neotmux/src/panes.rs crates/neotmux-core/src/panes.rs
git mv crates/neotmux/src/windows.rs crates/neotmux-core/src/windows.rs
```

In `crates/neotmux-core/src/lib.rs`:

```rust
pub mod layout;
pub mod panes;
pub mod windows;
```

In the moved `panes.rs`, `use neotmux_core::layout::{…}` becomes `use crate::layout::{…}`. In the moved `windows.rs`, `use crate::panes::PaneLayout;` is already right.

Remove `pub mod panes;` and `mod windows;` from `crates/neotmux/src/lib.rs`.

- [ ] **Step 2: Run the moved tests to verify they pass where they now live**

Run: `cargo test -p neotmux-core`
Expected: PASS — `layout`'s existing tests plus every test that moved with `panes.rs`.

- [ ] **Step 3: Point the binary's library at the new paths**

In `crates/neotmux/src/session.rs` and `crates/neotmux/src/shells.rs`, `use crate::panes::…` becomes `use neotmux_core::panes::…` and `use crate::windows::…` becomes `use neotmux_core::windows::…`.

- [ ] **Step 4: Check the boundary is real**

Run: `cargo tree -p neotmux-core --edges normal`
Expected: `neotmux-core` depends on `neotmux-proto` and nothing else — no `vt100`, no `portable-pty`, no `mlua`. This is §2's own check ("must stay testable with no terminal and no Lua"), asked of the dependency tree rather than of a person.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast`
Expected: clean, every suite green but the known flake.

- [ ] **Step 6: Commit**

```bash
git add crates
git commit -m "the pane tree and the windows over it, in the crate the design gave them to"
```

---

### Task 6: Take the edge out, and say so in the spec

The acceptance criterion for the whole plan, and not a judgement: the dependency comes out of `[dependencies]`, the workspace builds, and the edge is gone — or it does not build, and it is not.

`neotmux-term` moves to `[dev-dependencies]` rather than disappearing: `crates/neotmux-lua/tests/support/mod.rs` builds a real `PaneTerminals` as the host stand-in, and a fake would be a re-implementation of `vt100`. A dev-dependency is not an edge in the shipped graph, and the design's §8 should say which one it is.

**Files:**
- Modify: `crates/neotmux-lua/Cargo.toml`
- Modify: `docs/superpowers/specs/2026-09-19-neotmux-design.md` (§2, §2.1)
- Modify: `docs/superpowers/specs/2026-09-23-crate-boundaries-design.md` (§8, the dev-dependency)

**Interfaces:**
- Consumes: Tasks 1, 2 and 3 having removed every `[dependencies]` use of `neotmux-term` from the Lua crate.
- Produces: nothing later depends on.

- [ ] **Step 1: Move the dependency and watch it fail or not**

In `crates/neotmux-lua/Cargo.toml`, take `neotmux-term.workspace = true` out of `[dependencies]` and put it in `[dev-dependencies]`:

```toml
[dependencies]
neotmux-proto.workspace = true
anyhow.workspace = true
mlua.workspace = true

[dev-dependencies]
neotmux-term.workspace = true
tempfile.workspace = true
```

- [ ] **Step 2: Run the build — this is the acceptance test**

Run: `cargo build -p neotmux-lua`
Expected: PASS. Any `neotmux_term::` left in `crates/neotmux-lua/src/` is now a compile error naming itself.

- [ ] **Step 3: Confirm the graph**

Run: `cargo tree -p neotmux-lua --edges normal`
Expected: `neotmux-proto`, `mlua`, `anyhow` and their own trees. No `neotmux-term`, no `vt100`, no `portable-pty`, no `unicode-width` at the top level.

- [ ] **Step 4: Amend the design spec**

In `docs/superpowers/specs/2026-09-19-neotmux-design.md`:

Under §2's table, after the sentence about `neotmux-core` staying testable, add:

```markdown
The table's `neotmux-core` row became true on 2026-09-23, when `PaneLayout` and the window
set moved into it; before that it held layout alone. The same change took `neotmux-lua`'s
dependency on `neotmux-term` out of `[dependencies]`, which the "must not know about" column
had forbidden since this table was written. See
`docs/superpowers/specs/2026-09-23-crate-boundaries-design.md`.

Two entries in the row are still somewhere else and are meant to be. The event bus is
`neotmux-proto`'s `Event`, and the op buffer is `neotmux-lua`'s: M3 put the Lua layer on the
client, and the ops are the plugins'. The table is what is out of date there, not the code.
```

In §2.1, after the 2026-09-20 decision paragraph, add:

```markdown
That decision reached the reads on 2026-09-23. `Want`, `Trailing`, `Lines` and `Extent` had
one definition for the process and another for the wire, with a conversion between them; they
are now one each, here. `neotmux-proto` also holds `PaneRead`, and `width`, and one
dependency (`unicode-width`) to go with the latter. The rule that put the trait here and left
`Clipboard` in `neotmux-lua` is that **a contract lives in the lowest crate that both its
consumer and every one of its implementors can see**.
```

In `docs/superpowers/specs/2026-09-23-crate-boundaries-design.md` §8, after the dependency list, add:

```markdown
`neotmux-term` stays in `neotmux-lua`'s `[dev-dependencies]`. Its tests build a real
`PaneTerminals` as the host stand-in — a fake would be a re-implementation of `vt100` — and a
dev-dependency is not an edge in the shipped graph.
```

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast`
Expected: clean, every suite green but the known flake.

- [ ] **Step 6: Commit**

```bash
git add crates/neotmux-lua/Cargo.toml docs/superpowers/specs
git commit -m "the line comes out of the Cargo.toml, and the spec stops describing a graph that was not there"
```

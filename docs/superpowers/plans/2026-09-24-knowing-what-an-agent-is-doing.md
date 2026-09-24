# Knowing What An Agent Is Doing — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tell whether the Claude Code in a pane is working, waiting for you, or done — well
enough to make a sound about it.

**Architecture:** Two small host additions give Lua what it cannot otherwise see: a pane's
terminal title, through the `PaneRead` trait the Lua layer already reads panes with, and a
millisecond monotonic clock. On top of them, `neotmux.detect` is a rules engine over a *view* of
a pane — title, lines, and a few named regions — with a ruleset ported from herdr's Claude
manifest, and a hold that keeps a spinner's blink from reading as "done". `neotmux.state` keeps
its face and asks the detector first. `plugin.chime` watches the states and plays a sound.

**Tech Stack:** Rust 2024, mlua (Lua 5.4), `vt100` 0.16 (its `Callbacks` trait), and
`neotmux.job` for playing a sound. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-24-knowing-what-an-agent-is-doing-design.md`

## Global Constraints

- The detector is a pure function of a view: no `neotmux.*` call inside a rule, so every rule is
  testable as screens in, states out.
- Rules are sorted by priority, highest first; the first match answers. A rule whose `state` is
  `nil` means "this screen is not a state" and leaves the previous answer standing.
- A fall from `working` to `idle` is held for `detect.settle` (700) milliseconds unless the new
  answer is *visibly* idle or blocked. Every other transition is immediate.
- `state.of`, `state.worst`, `state.icon`, `state.icons` and `state.forget` keep their
  signatures. A pane no ruleset claims falls back to the existing pattern list, unchanged.
- `plugin.chime` is silent for the focused pane by default (`only_when_away = true`), and plays
  one sound per *entry* into a state.
- A player is judged by how it exits, never by whether it started: `aplay` starts happily and
  plays an mp3 as noise. The list is herdr's, in its order, and `aplay` is not in it.
- The rules, the hold and the two sounds are derived from herdrdev/herdr (Apache-2.0).
  Attribution goes in `runtime/lua/neotmux/detect.lua`'s header, beside the sounds, and in a new
  `NOTICE` at the repository root.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` are all green before a task is done.
- Comments explain *why*, never *what*. Match the surrounding files' density and voice.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/neotmux-term/src/state.rs` | keeps the last OSC 0/2 title beside the screen |
| `crates/neotmux-term/src/terminals.rs` | `title(pane)`, and the `PaneRead` impl for it |
| `crates/neotmux-proto/src/read.rs` | `PaneRead::title` — the seam, not the wire |
| `crates/neotmux-lua/src/api/pane.rs` | `neotmux.pane.title(id)` |
| `crates/neotmux-lua/src/api/mod.rs` | `neotmux.now()` |
| `runtime/lua/neotmux/detect.lua` (new) | the view, the rule engine, the hold, `detect.agents` |
| `runtime/lua/neotmux/agents/claude.lua` (new) | the Claude ruleset, one table |
| `runtime/lua/neotmux/state.lua` | asks the detector, falls back to its own patterns |
| `runtime/lua/plugin/chime.lua` (new) | a sound on a state change |
| `runtime/sounds/{done,request}.mp3` (new) | herdr's, carried with attribution |
| `NOTICE` (new) | what this repository carries that is somebody else's |
| `crates/neotmux-lua/tests/detect.rs` (new) | screens in, states out |
| `crates/neotmux-lua/tests/chime.rs` (new) | states in, players tried, one sound per entry |
| `docs/plugins.md` | the detect convention, `pane.title`, `neotmux.now`, `plugin.chime` |

---

## Task 1: A pane's title

**Files:**
- Modify: `crates/neotmux-term/src/state.rs`, `crates/neotmux-term/src/terminals.rs`,
  `crates/neotmux-proto/src/read.rs`, `crates/neotmux-lua/src/api/pane.rs`
- Test: in `crates/neotmux-term/src/state.rs`'s own `mod tests`, and
  `crates/neotmux-lua/tests/pane.rs` if one exists — otherwise the crate's nearest pane test

**Interfaces:**
- Consumes: nothing.
- Produces: `PaneRead::title(&self, pane: PaneId) -> Option<String>`;
  `TermState::title(&self) -> Option<&str>`; `PaneTerminals::title(pane)`;
  `neotmux.pane.title(id)` in Lua.

**A ruling to carry:** the spec says `pane.info(id).title`. `PaneInfo` is what `/proc` says about
a pane's process, cached with an age, and published per frame; a title is what the *program*
said, and it reaches Lua through `PaneRead` like every other fact about a terminal. So it is
`neotmux.pane.title(id)`, a direct call beside `neotmux.pane.revision(id)`, and nothing goes on
the wire. Amend the spec's §2.1 to match in this task's commit.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux-term/src/state.rs`'s `mod tests`:

```rust
    /// A program's title is a statement it made about itself, and the one thing on a screen
    /// that survives the screen scrolling away. `vt100` hands it over only through a callback,
    /// so this is a test that the callback is wired at all.
    #[test]
    fn a_pane_remembers_the_last_title_its_program_set() {
        let mut state = TermState::new(Size { rows: 4, cols: 20 }, 10);
        assert_eq!(state.title(), None);

        state.feed(b"\x1b]2;first\x07");
        assert_eq!(state.title(), Some("first"));

        // OSC 0 sets the icon name and the title together; both forms are what a program uses.
        state.feed(b"\x1b]0;second\x07");
        assert_eq!(state.title(), Some("second"));

        // A title is not text: it does not land on the screen.
        assert!(!crate::lines::visible(&state).iter().any(|l| l.contains("second")));
    }
```

Check `TermState`'s constructor signature and the helper for reading visible lines before
writing this — use whatever the neighbouring tests in that file use.

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux-term state::tests::a_pane_remembers`
Expected: FAIL — no `title` method.

- [ ] **Step 3: Give the parser somewhere to put it**

`vt100::Parser::new` takes no callbacks; `vt100::Parser::new_with_callbacks(rows, cols,
scrollback, callbacks)` does, and `parser.callbacks()` reads them back. So `TermState`'s
`parser` field becomes `vt100::Parser<Titles>`:

```rust
/// What the program said its window is called. `vt100` parses OSC 0 and OSC 2 and then asks
/// what to do with them; before this it was asked and answered nothing.
#[derive(Default)]
pub(crate) struct Titles {
    title: Option<String>,
}

impl vt100::Callbacks for Titles {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        // A title is whatever bytes the program sent. Lossy rather than refused: a title that
        // is not UTF-8 is still a title, and the alternative is to forget it entirely.
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }
}
```

`TermState::new` uses `new_with_callbacks(size.rows, size.cols, scrollback, Titles::default())`,
and:

```rust
    /// The last title the program set, or `None` for one that never set any.
    pub fn title(&self) -> Option<&str> {
        self.parser.callbacks().title.as_deref()
    }
```

Every other use of `self.parser` keeps working — the type parameter does not change the methods.

- [ ] **Step 4: Carry it up to Lua**

`PaneTerminals::title(&self, pane: PaneId) -> Option<String>` in `terminals.rs`, in the shape of
the `revision` beside it; add `fn title(&self, pane: PaneId) -> Option<String>;` to `PaneRead`
in `read.rs` with a doc comment saying what a title is and that it is the program's own claim;
implement it in the `impl PaneRead for PaneTerminals` block.

In `crates/neotmux-lua/src/api/pane.rs`, beside `revision`:

```rust
    // The title, not the command: a program that sets one is saying what it is doing, which
    // is a different fact from what `pane.info` reads out of `/proc`.
    let title = lua.create_function(move |_, id: u32| {
        Ok(panes.title(PaneId::from(id)))
    })?;
    table.set("title", title)?;
```

Follow the file's own idiom for taking `panes` and validating the id — copy `revision`'s shape
exactly, including what it does for a pane that does not exist.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux-term && cargo test -p neotmux-lua`
Expected: PASS.

- [ ] **Step 6: Note it in the docs and amend the spec**

`docs/plugins.md`: `neotmux.pane.title(id)` goes wherever `pane.revision` is documented, with a
sentence that it is what the program said, `nil` for one that never said anything.
`docs/superpowers/specs/2026-09-24-knowing-what-an-agent-is-doing-design.md` §2.1: the call is
`neotmux.pane.title(id)`, not `pane.info(id).title`, and say why in one sentence.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a pane remembers what its program calls itself"
```

---

## Task 2: A clock in milliseconds

**Files:**
- Modify: `crates/neotmux-lua/src/api/mod.rs`, `docs/plugins.md`
- Test: `crates/neotmux-lua/tests/subsystem.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `neotmux.now()` — a Lua number, milliseconds, monotonic.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux-lua/tests/subsystem.rs`, following that file's harness:

```rust
/// The only clock a plugin had counted whole seconds, which is too coarse for anything that
/// happens between two frames. Monotonic and relative: what it is zeroed at is nobody's
/// business, and only differences mean anything.
#[test]
fn the_clock_answers_milliseconds_that_never_go_backwards() {
    let (_dir, sub) = built(
        "local first = neotmux.now()\n\
         local last = first\n\
         for _ = 1, 1000 do\n\
           local now = neotmux.now()\n\
           if now < last then\n\
             error('the clock went backwards')\n\
           end\n\
           last = now\n\
         end\n\
         neotmux.log.info('span=' .. tostring(last - first >= 0))\n\
         neotmux.log.info('number=' .. type(first))\n",
    );
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "span=true"), "{said:?}");
    assert!(said.iter().any(|line| line == "number=number"), "{said:?}");
}
```

Also add `now` to the sealed-ABI test in that file that pins the top-level field list — it is
sorted, so `now` goes between `notify` and `opt` (check the actual list).

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p neotmux-lua --test subsystem the_clock_answers`
Expected: FAIL — `neotmux.now` is nil.

- [ ] **Step 3: Install it**

In `api/mod.rs`'s `install`, beside the other plain functions:

```rust
    {
        // Zeroed when the subsystem is built rather than at the epoch: a `Duration` since an
        // arbitrary start is what a monotonic clock can honestly answer, and milliseconds are
        // what every timing a plugin does is written in.
        let start = std::time::Instant::now();
        let now = lua.create_function(move |_, ()| {
            Ok(start.elapsed().as_millis() as u64)
        })?;
        neotmux.set("now", now)?;
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --test subsystem`
Expected: PASS.

- [ ] **Step 5: Document it**

`docs/plugins.md`, wherever the top-level `neotmux.*` calls are listed: `neotmux.now()` is
milliseconds from an arbitrary zero, monotonic, and only differences mean anything. Say that
`os.time` is still there and still counts seconds.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a plugin can tell how long a thing took"
```

---

## Task 3: The view and the rule engine

**Files:**
- Create: `runtime/lua/neotmux/detect.lua`, `crates/neotmux-lua/tests/detect.rs`
- Modify: `runtime/init.lua` only if it enumerates convention modules (check; it may not)

**Interfaces:**
- Consumes: `neotmux.pane.title(id)` (Task 1), `neotmux.now()` (Task 2).
- Produces: `detect.view(pane) -> view`, `detect.judge(ruleset, view) -> state, rule_id, visible`,
  `detect.of(pane) -> 'working'|'blocked'|'idle'|nil`, `detect.agents` (a table of rulesets, empty
  until Task 4), `detect.settle = 700`, `detect.forget(pane)`.

No rules yet — this task is the machine, tested with rulesets the test writes itself.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/detect.rs`. Build the subsystem the way `crates/neotmux-lua/tests/state.rs`
does if it exists, otherwise copy `tests/hints.rs`'s harness. Feed a pane real bytes with
`sub.terminals().feed(...)` so the view is built from a real screen.

```rust
//! `neotmux.detect`: what a pane looks like it is doing, as a function of what is on it.
//! The rules under test here are the tests' own — the shipped Claude ruleset has its own file.

/// Priority decides, not order in the table: a lower-priority rule that matches first in the
/// table must not win.
#[test]
fn the_highest_priority_matching_rule_answers() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         local rules = {\n\
           { id = 'low', state = 'idle', priority = 1, match = function() return true end },\n\
           { id = 'high', state = 'blocked', priority = 9, match = function() return true end },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local state, id = detect.judge(rules, detect.view(1))\n\
             neotmux.log.info('state=' .. tostring(state) .. ' id=' .. tostring(id))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    keys(&sub, &["J"]);
    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "state=blocked id=high"), "{said:?}");
}

/// A rule with no state says "this screen is not a state" — a transcript viewer, a menu — and
/// stops the search without answering, so the caller keeps whatever it had.
#[test]
fn a_rule_with_no_state_stops_the_search_without_answering() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         local rules = {\n\
           { id = 'viewer', priority = 9, match = function() return true end },\n\
           { id = 'idle', state = 'idle', priority = 1, match = function() return true end },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local state, id = detect.judge(rules, detect.view(1))\n\
             neotmux.log.info('state=' .. tostring(state) .. ' id=' .. tostring(id))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    keys(&sub, &["J"]);
    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "state=nil id=viewer"), "{said:?}");
}

/// The view's regions, against one screen that has all three: a rule above, a box below, and
/// blank lines that no region counts.
#[test]
fn the_view_names_the_regions_a_rule_reads() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local view = detect.view(1)\n\
             neotmux.log.info('tail=' .. table.concat(view.tail(2), '|'))\n\
             neotmux.log.info('after=' .. table.concat(view.after_rule(), '|'))\n\
             neotmux.log.info('box=' .. table.concat(view.prompt_box(), '|'))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(
        &sub,
        "above\r\n\
         \u{2500}\u{2500}\u{2500}\u{2500}\r\n\
         between\r\n\
         \r\n\
         \u{256d}\u{2500}\u{2500}\u{256e}\r\n\
         \u{2502} \u{276f} \u{2502}\r\n\
         \u{2570}\u{2500}\u{2500}\u{256f}"
            .as_bytes(),
    );
    keys(&sub, &["J"]);
    let said = logged(&sub);
    // The blank line is in none of them: every region is built from the non-blank lines.
    assert!(
        said.iter()
            .any(|l| l == "tail=\u{2502} \u{276f} \u{2502}|\u{2570}\u{2500}\u{2500}\u{256f}"),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.starts_with("after=between|")),
        "the rule line itself or the blank leaked into after_rule: {said:?}"
    );
    assert!(said.iter().any(|l| l == "box=\u{276f}"), "{said:?}");
}

/// Held: a fall from working to plain idle is not believed at once. The rules are the test's,
/// flipped by a value it controls, so the only thing under test is the hold.
#[test]
fn a_working_pane_that_looks_idle_is_still_working_until_it_settles() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         detect.settle = 50\n\
         local says = 'working'\n\
         detect.agents = {\n\
           test = {\n\
             { id = 'spinner', state = 'working', priority = 9, visible = 'working',\n\
               match = function() return says == 'working' end },\n\
             { id = 'quiet', state = 'idle', priority = 1,\n\
               match = function() return says == 'idle' end },\n\
           },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'W' then says = 'working' return true end\n\
           if key == 'I' then says = 'idle' return true end\n\
           if key == 'J' then\n\
             detect.forget(1)\n\
             neotmux.log.info('of=' .. tostring(detect.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    keys(&sub, &["W", "J"]);
    assert!(logged(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["I", "J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=working"),
        "a working pane fell to idle the moment its spinner blinked"
    );

    // Past `settle`, with nothing new to say for itself, it is idle.
    std::thread::sleep(std::time::Duration::from_millis(60));
    keys(&sub, &["J"]);
    assert!(logged(&sub).iter().any(|l| l == "of=idle"));
}

/// Unless the screen is *visibly* idle, which is evidence rather than an absence of it.
#[test]
fn a_visible_idle_prompt_is_believed_at_once() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         detect.settle = 5000\n\
         local says = 'working'\n\
         detect.agents = {\n\
           test = {\n\
             { id = 'spinner', state = 'working', priority = 9, visible = 'working',\n\
               match = function() return says == 'working' end },\n\
             { id = 'prompt', state = 'idle', priority = 1, visible = 'idle',\n\
               match = function() return says == 'idle' end },\n\
           },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'W' then says = 'working' return true end\n\
           if key == 'I' then says = 'idle' return true end\n\
           if key == 'J' then\n\
             detect.forget(1)\n\
             neotmux.log.info('of=' .. tostring(detect.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    keys(&sub, &["W", "J"]);
    assert!(logged(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["I", "J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=idle"),
        "a prompt on the screen is evidence, and was held anyway"
    );
}
```

`detect.forget(1)` before each read is deliberate: `of` caches by the pane's revision, and the
screen is not changing between these reads — only the test's own flag is.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test detect`
Expected: FAIL — no `neotmux.detect`.

- [ ] **Step 3: Write the module**

`runtime/lua/neotmux/detect.lua`. Its header carries the attribution (see Global Constraints)
and explains the split: the engine is here, the knowledge is in `neotmux/agents/*.lua`.

The view:

```lua
-- What a rule reads. Built once per look and handed to every rule, with each region computed
-- on first ask: a rule that never looks at the prompt box costs nothing for it.
function M.view(pane)
  local lines = neotmux.pane.content(pane)
  if lines == nil then
    return nil
  end
  local full = {}
  for _, line in ipairs(lines) do
    if line:match("%S") then
      full[#full + 1] = line
    end
  end
  ...
end
```

`view.title` is `neotmux.pane.title(pane) or ""`. `view.lines` is the non-blank lines.
`view.tail(n)` is the last `n` of them. `view.after_rule()` is everything after the last line
that is a horizontal rule (a line whose only non-space characters are `─` or `-`, at least
three of them), or all of `lines` when there is none. `view.prompt_box()` is the lines between
the last `╭`-started line and the `╰` that closes it, stripped of their `│` edges, or an empty
table. Memoise each behind a local.

`M.judge(rules, view)` sorts a copy by priority descending — sort once and cache per ruleset
table, since a ruleset does not change between looks — runs each `match(view)` behind `pcall`,
and answers `state, id, visible` for the first that returns true. A rule that raises is reported
with `neotmux.notify` naming its id and skipped, because one bad rule must not take the detector
down.

`M.of(pane)` builds the view, tries each ruleset in `M.agents` until one answers, applies the
hold, and caches by `neotmux.pane.revision(pane)` the way `neotmux.state` already does. The hold
is a table of `{ state, since }` per pane, compared against `neotmux.now()`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --test detect`
Expected: PASS, all five.

- [ ] **Step 5: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a rule reads a view of a pane, and the highest one answers"
```

---

## Task 4: The Claude ruleset

**Files:**
- Create: `runtime/lua/neotmux/agents/claude.lua`
- Modify: `runtime/lua/neotmux/detect.lua` (register it in `M.agents`),
  `crates/neotmux-lua/tests/detect.rs`

**Interfaces:**
- Consumes: the view and `judge` from Task 3.
- Produces: `detect.agents.claude` — a list of rules, each `{ id, state, priority, visible?, match }`.

The rules below are the port of herdr's `claude.toml`. Keep the ids and priorities: they are how
a reader compares this table against the manifest it came from.

| priority | id | state | visible | what it matches |
|---|---|---|---|---|
| 1100 | `osc_title_working` | working | working | `view.title` begins with a braille character (U+2800–U+28FF) or a half-circle (U+25D0–U+25D3), followed by a space |
| 1000 | `transcript_viewer` | — | — | the last 3 lines contain "showing detailed transcript" and one of "ctrl+o", "ctrl+e", "↑↓ scroll", "? for shortcuts" |
| 980 | `live_blocked_form` | blocked | blocked | `after_rule()` contains "esc to cancel" **and** ("enter to confirm" or ("enter to select" with a navigation hint)) |
| 975 | `btw_overlay_working` | working | working | the last 5 lines have one starting `/btw`, or one ending "esc to close" |
| 970 | `live_turn_working` | working | working | in the last 12: a line matching `^%s*[⏸⏵].*esc to interrupt`, or one starting with a spinner glyph (`*`, `·`, `✢`, `✳`, `✶`, `✻`, `✽`) and containing `…` |
| 950 | `live_prompt_box` | idle | idle | `prompt_box()` has a line starting `❯`, and the box holds none of "enter to select", "esc to cancel", "arrow keys", "↑/↓ to navigate" |
| 900 | `model_picker_menu` | — | — | anywhere: "select model" and "enter to set as default" and "esc to cancel", without "do you want to proceed?" |
| 850 | `bash_permission_prompt` | blocked | blocked | "do you want to proceed?" plus one of "bash command", "bash(", "contains expansion", "tab to amend", "ctrl+e to explain", plus a line matching a yes/no option: `^%s*❯?%s*%d?%.?%s*[Yy]es` or `^%s*❯?%s*%d?%.?%s*[Nn]o` |
| 840 | `generic_permission_prompt` | blocked | blocked | `after_rule()` has "do you want to proceed?" and "esc to cancel" and a numbered yes/no option line |
| 300 | `legacy_no_prompt_blocker` | blocked | — | any of: "do you want to" with "yes" or `❯`; "would you like to" likewise; "waiting for permission"; "do you want to allow this connection?"; "tab to amend"; "ctrl+e to explain"; "review your answers" — and **not** a line that is only `❯` |
| 250 | `osc_title_idle` | idle | idle | `view.title` begins with `✳` and a space |

Two of herdr's rules are deliberately not ported (`background_agents_working`,
`background_mcp_task_working`): they read shapes nobody here can test against a real screen.
Say so in the file.

**On matching:** every comparison of screen text is case-insensitive — lower-case the line once
per rule rather than per comparison. Lua patterns are bytes, so a braille character is three
bytes; match the title's first character with `utf8.codepoint` and a range comparison rather
than with a pattern.

- [ ] **Step 1: Write the failing tests**

Add to `crates/neotmux-lua/tests/detect.rs` one test per rule, each feeding a screen and
asserting **both** the state and the rule id that fired. The fixtures below are the shapes the
manifest describes; check each against a real Claude Code screen where you can, and say in your
report which ones you could not check.

```rust
/// A working turn: the spinner line Claude draws while it is going, with its interrupt hint.
#[test]
fn a_turn_in_progress_is_working() {
    let screen = "✻ Herding… (12s · esc to interrupt)\n";
    assert_detects(screen, "", "working", "live_turn_working");
}

/// The title outranks the screen: a screen that reads idle under a working title is working.
#[test]
fn the_title_outranks_the_screen() {
    let screen = "╭────────╮\n│ ❯      │\n╰────────╯\n";
    assert_detects(screen, "⠋ Claude", "working", "osc_title_working");
}

/// A Bash approval names the command and offers numbered options.
#[test]
fn a_bash_permission_prompt_is_blocked() {
    let screen = "Bash command\n\
                  ls -la\n\
                  Do you want to proceed?\n\
                  ❯ 1. Yes\n\
                    2. Yes, and don't ask again for: ls\n\
                    3. No\n";
    assert_detects(screen, "", "blocked", "bash_permission_prompt");
}
```

Write `assert_detects(screen, title, state, rule_id)` as the file's own helper: feed the screen
to the pane, set the title by feeding `\x1b]2;<title>\x07`, and read back what `detect.judge`
answered. Cover every row of the table, including the two `—` rows, which assert that the state
is `nil` and the id is the viewer's.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test detect`
Expected: FAIL — `detect.agents.claude` does not exist.

- [ ] **Step 3: Write the ruleset**

`runtime/lua/neotmux/agents/claude.lua` returns the list. Each rule's `match` is a small
function over the view; keep the helpers (`any_line`, `holds`, `lower`) local to the file.
Register it in `detect.lua`: `M.agents = { claude = require('neotmux.agents.claude') }`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --test detect`
Expected: PASS, every rule.

- [ ] **Step 5: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "what a claude code screen means, eleven rules of it"
```

---

## Task 5: `neotmux.state` asks the detector

**Files:**
- Modify: `runtime/lua/neotmux/state.lua`, `docs/plugins.md`
- Test: `crates/neotmux-lua/tests/state.rs` if it exists, else add the cases to `tests/detect.rs`

**Interfaces:**
- Consumes: `detect.of(pane)`.
- Produces: nothing new — `state.of` and friends keep their signatures exactly.

- [ ] **Step 1: Write the failing tests**

```rust
/// A Claude screen gets the detector's answer. The pattern list would call this idle — there
/// is no question mark on it — and the detector calls it working, which is the point.
#[test]
fn a_pane_a_ruleset_claims_gets_the_detectors_answer() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, "\u{273b} Herding… (12s \u{00b7} esc to interrupt)".as_bytes());
    keys(&sub, &["J"]);
    assert!(logged(&sub).iter().any(|l| l == "of=working"), "the detector was not asked");
}

/// A pane no ruleset claims still gets the old guess: `[y/N]` is a question, and nothing in the
/// Claude ruleset has anything to say about a shell.
#[test]
fn a_pane_no_ruleset_claims_falls_back_to_the_patterns() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"rm -i file\r\nremove file? [y/N] ");
    keys(&sub, &["J"]);
    assert!(logged(&sub).iter().any(|l| l == "of=blocked"), "the fallback stopped working");
}

/// And the fallback's "anything that printed just now is working" still holds, in milliseconds
/// against the new clock rather than seconds against `os.time`.
#[test]
fn the_fallback_still_calls_a_pane_that_just_printed_working() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         state.working_for = 5000\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"building something");
    keys(&sub, &["J"]);
    assert!(logged(&sub).iter().any(|l| l == "of=working"), "a pane that just printed is working");
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test state` (or `--test detect`)
Expected: FAIL.

- [ ] **Step 3: Rewire `of`**

In `state.lua`, `M.of(pane)` asks `require('neotmux.detect').of(pane)` first and returns it when
it is not nil. The existing body — `asking()`, the pattern list, `M.working_for` — becomes the
fallback, unchanged except that `M.working_for` is now milliseconds against `neotmux.now()`
rather than seconds against `os.time()`. Update its doc comment and its default (2 → 2000).

`M.forget(pane)` also forgets the detector's memory of that pane.

Keep the module header honest: it says the three words are herdr's; it now also says where the
detector lives and that this file is what a pane nobody has a ruleset for gets.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua`
Expected: PASS.

- [ ] **Step 5: Document it**

`docs/plugins.md`'s state convention section: the detector is where the guess lives now, the
pattern list is the fallback, and `neotmux.detect.agents` is where a ruleset for another agent
goes.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "the state of a pane is the detector's answer where there is one"
```

---

## Task 6: `plugin.chime`

**Files:**
- Create: `runtime/lua/plugin/chime.lua`, `runtime/sounds/done.mp3`,
  `runtime/sounds/request.mp3`, `NOTICE`, `crates/neotmux-lua/tests/chime.rs`
- Modify: `runtime/lua/neotmux/defaults.lua`, `runtime/init.lua`, `docs/plugins.md`

**Interfaces:**
- Consumes: `state.of(pane)`, `neotmux.job.start`, `neotmux.pane.list()`, `neotmux.pane.current()`.
- Produces: `plugin.chime` with `sounds`, `players`, `only_when_away`.

- [ ] **Step 1: Fetch the sounds and write the NOTICE**

```bash
mkdir -p runtime/sounds
gh api repos/herdrdev/herdr/contents/assets/sounds/done.mp3 --jq '.content' | base64 -d > runtime/sounds/done.mp3
gh api repos/herdrdev/herdr/contents/assets/sounds/request.mp3 --jq '.content' | base64 -d > runtime/sounds/request.mp3
file runtime/sounds/*.mp3
```

`NOTICE` at the repository root says what is carried and from where: the two sounds, and the
detection rules and hold in `neotmux/detect.lua` and `neotmux/agents/claude.lua`, all derived
from herdrdev/herdr under Apache-2.0, with a pointer to `LICENSE-APACHE`.

- [ ] **Step 2: Write the failing tests**

`crates/neotmux-lua/tests/chime.rs`, with the `FakeRunner` from `crates/neotmux-lua/tests/jobs.rs`
(copy it; the two files are separate test binaries and cannot share a module unless you put it in
`tests/support/`, which is the better move if it is a clean lift).

```rust
/// One sound per entry into a state: a pane that is still blocked next frame does not chime
/// again, and one that goes back to working and blocks again does.
#[test]
fn a_pane_chimes_when_it_starts_waiting_and_not_again() {
    let runner = FakeRunner::new();
    // Two panes, so the one under test is never the focused one.
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));
    blocked(&sub, TWO);
    frame(&sub);
    assert_eq!(runner.started.borrow().len(), 1, "no sound for a pane that started waiting");
    assert!(runner.started.borrow()[0].cmd.last().unwrap().ends_with("request.mp3"));

    frame(&sub);
    assert_eq!(runner.started.borrow().len(), 1, "it chimed again for the same wait");

    working(&sub, TWO);
    frame(&sub);
    blocked(&sub, TWO);
    frame(&sub);
    assert_eq!(runner.started.borrow().len(), 2, "a second wait is a second sound");
}

/// The pane you are looking at does not make a noise at you.
#[test]
fn the_focused_pane_is_silent_by_default() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));
    blocked(&sub, ONE); // ONE is the focused pane in the harness's geometry
    frame(&sub);
    assert!(runner.started.borrow().is_empty(), "the pane in front of you chimed");
}

/// A player is judged by how it exits: one that will not start is skipped, one that starts and
/// exits non-zero is skipped, and the first that exits zero is remembered — the ones before it
/// are not tried again.
#[test]
fn the_first_player_that_exits_zero_is_the_one_from_then_on() {
    let runner = FakeRunner::new();
    runner.refuse_program("paplay"); // not on this machine
    runner.will_say(vec![vec![JobChunk::Exited(1)]]); // pw-play: starts, cannot decode
    runner.will_say(vec![vec![JobChunk::Exited(0)]]); // ffplay: works
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));

    blocked(&sub, TWO);
    frame(&sub); // pw-play starts and fails
    frame(&sub); // its on_exit moves on to ffplay
    frame(&sub);
    let tried: Vec<String> = runner
        .started
        .borrow()
        .iter()
        .map(|spec| spec.cmd[0].clone())
        .collect();
    assert_eq!(tried, vec!["pw-play", "ffplay"], "{tried:?}");

    // The next chime goes straight to the one that worked.
    working(&sub, TWO);
    frame(&sub);
    blocked(&sub, TWO);
    frame(&sub);
    let tried: Vec<String> = runner
        .started
        .borrow()
        .iter()
        .map(|spec| spec.cmd[0].clone())
        .collect();
    assert_eq!(tried, vec!["pw-play", "ffplay", "ffplay"], "{tried:?}");
}

/// When none of them work, it says so once and stops.
#[test]
fn a_machine_with_no_player_is_told_once() {
    let runner = FakeRunner::new();
    runner.refuse_all();
    let (_dir, sub) = built(TWO_PANES, Rc::clone(&runner));

    blocked(&sub, TWO);
    frame(&sub);
    let said = notified(&sub);
    assert_eq!(
        said.iter().filter(|line| line.contains("no player")).count(),
        1,
        "{said:?}"
    );

    working(&sub, TWO);
    frame(&sub);
    blocked(&sub, TWO);
    frame(&sub);
    assert!(
        notified(&sub).iter().all(|line| !line.contains("no player")),
        "it kept trying after it had given up"
    );
}
```

`TWO_PANES` is an `init.lua` that leaves `plugin.chime` on and every other plugin off; `ONE` and
`TWO` are the two pane ids the harness publishes geometry for, with `ONE` focused. `blocked(&sub,
pane)` and `working(&sub, pane)` feed that pane a screen the Claude ruleset reads — the
permission prompt and the spinner line from Task 4's fixtures — so the test drives the real path
rather than a stubbed `state.of`. `FakeRunner` gains `refuse_program(name)` and `refuse_all()`
beside the `refuse` field it already has.

- [ ] **Step 3: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test chime`
Expected: FAIL — no `plugin.chime`.

- [ ] **Step 4: Write the plugin**

```lua
M.sounds = { blocked = "sounds/request.mp3", idle = "sounds/done.mp3" }
M.players = {
  { "paplay" },
  { "pw-play" },
  { "ffplay", "-nodisp", "-autoexit", "-loglevel", "quiet" },
  { "mpg123", "-q" },
  { "mpv", "--no-video", "--really-quiet" },
}
M.only_when_away = true
```

A path is resolved against the layer the plugin came from — `neotmux.home` is not it; use the
runtime layer's own directory, which `require`'s loader knows. If there is no way to ask for it,
make `M.sounds` absolute paths built once at load from a `M.dir` the file sets, and say so.

`on_tick`: for each pane of the current window (`neotmux.pane.list()`), read `state.of`, compare
against what was last seen for that pane, and on a change *into* a state with a sound, play it —
unless `only_when_away` and the pane is `neotmux.pane.current()`.

Playing walks `M.players` from a remembered index: `neotmux.job.start({ program, args..., path })`,
and in `on_exit`, a non-zero code or a nil code means "try the next one" while a zero code means
"remember this index". When the walk runs out, `neotmux.notify('error', ...)` once and set a flag
that stops further attempts.

Register it in `runtime/lua/neotmux/defaults.lua` and `runtime/init.lua` the way the other
plugins are, **on by default** — the sounds ship, so it works out of the box.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux-lua --test chime`
Expected: PASS.

- [ ] **Step 6: Exercise it for real**

Build, run a session, and make a Claude Code pane go from working to waiting. Say in your report
what you heard, which player won, and what `neotmux.log` said. If there is no Claude Code to
hand, drive it with a shell script that prints the fixture screens and sets the titles.

- [ ] **Step 7: Document it**

`docs/plugins.md`: `plugin.chime` in the plugin table and a section of its own — the two options,
why a player is judged by its exit rather than its start, where the sounds came from, and that
it is silent for the focused pane. Correct the plugin count line in that file.

`CHANGELOG.md` under `## Unreleased`: a prose entry in the voice of the ones around it, covering
the detector and the chime together.

- [ ] **Step 8: Verify and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -m "a pane that starts waiting for you says so out loud"
```

---

## Self-review against the spec

| Spec section | Task |
|---|---|
| §2.1 a pane's title | 1 (as `pane.title(id)`, with the spec amended and the ruling recorded) |
| §2.2 `neotmux.now()` | 2 |
| §3.1 the view and its regions | 3 |
| §3.2 rules, priority, the `nil` state | 3 |
| §3.3 the hold | 3 |
| §3.4 the Claude ruleset | 4 |
| §4 `neotmux.state` keeps its face | 5 |
| §5 `plugin.chime`, §5.1 finding a player | 6 |
| §6 what is tested | every task; the fixtures are Task 4's, the player walk is Task 6's |
| attribution and NOTICE | 6 (the file), 3 and 4 (the headers) |

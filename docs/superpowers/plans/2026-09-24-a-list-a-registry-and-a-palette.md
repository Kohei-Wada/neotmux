# A list, a registry, and a palette — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `C-b :` opens a command palette — a float with an input line, a fuzzy-filtered list of
every registered command, and Enter to run one.

**Architecture:** Three pieces, in the order they depend on each other. `neotmux.list` is a
keyboard-driven list in a float, a Lua convention beside `neotmux.prompt` and `neotmux.box`;
the shipped session picker is rebuilt on it, which is the proof it is sufficient.
`neotmux.command` is a host-side registry of named callbacks a plugin can enumerate and call.
`plugin.palette` is the composition of the two, and is small because they exist.

**Tech Stack:** Rust 2024 for the registry (mlua, the existing `Registry`/`Sources`), Lua 5.4
for the conventions and plugins. No new dependencies.

**Specs:** `docs/superpowers/specs/2026-09-23-the-command-surface-design.md` §4 (what a plugin
command is) — this plan builds its in-process half only. Issue #35 is the argument for
`neotmux.list`; issue #18 is the palette.

## Global Constraints

- **Conventions are Lua, mechanisms are the host.** `neotmux.list` and `plugin.palette` are
  files a user can replace wholesale with no change to the host. The registry is the host's,
  because a name has to be resolvable without running Lua when the CLI reaches it later.
- **The rebuild must not change what the session picker does or looks like.** Task 2 writes the
  test against the current implementation first and it must pass unchanged afterwards.
- Every new module is documented in its own header the way `neotmux/box.lua` and
  `neotmux/mode.lua` are: what it is for, why it is not a host feature, and the one example.
- Keys follow the shipped vim four: `j`/`k` move, `Enter` chooses, `Esc`/`q` leave. No arrows.
- Every task ends green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test --workspace`. Two PTY tests
  (`socket::ending_one_session_leaves_the_other_typing_into_its_own_shell`,
  `terminal_restore::renders_output_written_in_the_same_batch_as_exit`) flake under parallel
  load — re-run alone before blaming the branch.
- Commit messages are prose in this repo's voice: a lower-case summary line that says what is
  true now, then why. No `feat:` prefixes, no Co-Authored-By trailers.
- No CLI work. `neotmux <plugin-command>` from outside is the command surface spec's §5/§6 and
  is deliberately not in this plan.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `runtime/lua/neotmux/list.lua` | **new** — a keyboard-driven list in a float: selection, scrolling, an optional input line, and the drawing |
| `runtime/lua/plugin/sessions.lua` | rebuilt as a composition of `neotmux.list`, same behaviour |
| `crates/neotmux-lua/src/commands.rs` | **new** — the registry: name → callback, source, description |
| `crates/neotmux-lua/src/api/command.rs` | **new** — `neotmux.command.add/list/run` |
| `crates/neotmux-lua/src/lib.rs` | wiring, and `run_command` for a host caller later |
| `runtime/lua/plugin/prefix.lua` | its verbs registered as named commands, its keys calling them |
| `runtime/lua/plugin/palette.lua` | **new** — `C-b :`, fuzzy over the registry, Enter runs |
| `docs/plugins.md` | the list convention, the command registry, the palette |

---

### Task 1: `neotmux.list`

**Files:**
- Create: `runtime/lua/neotmux/list.lua`
- Test: `crates/neotmux-lua/tests/list.rs` (create)

**Interfaces:**
- Produces `require('neotmux.list')` with one function:

```lua
local handle = list.open({
  title  = 'sessions',            -- drawn in the frame's top line; optional
  mode   = 'sessions',            -- raised through neotmux.mode while up; optional
  width  = 40,                    -- the most columns it takes (default 40)
  rows   = 10,                    -- the most rows of list it takes (default 10)
  items  = function() end,        -- called on every tick and every key: the rows, fresh
  row    = function(item, chosen) end,  -- runs for one row: { { text =, fg =, bg = }, ... }
  footer = function() end,        -- runs for the last row when no input is up; optional
  keys   = { ['c'] = function(item, handle) end },  -- extra keys; optional
  done   = function(item, handle) end,  -- Enter on a row
  cancel = function() end,        -- Esc or q; optional
  filter = function(item, query) end,   -- optional: turns the input line on, and answers
                                        -- whether this item survives the query
})
```

and on the handle:

| call | what it does |
| --- | --- |
| `handle:close()` | takes the float down, lowers the mode, forgets the state. Idempotent. |
| `handle:selected()` | the item under the cursor, or nil |
| `handle:ask(opts)` | turns the footer into an input line: `{ label =, value =, done = fn, cancel = fn }`. Enter calls `done(text)`, Esc calls `cancel`. |
| `handle:query()` | what is typed in the filter line, or nil where `filter` was not given |

One list is open at a time; opening another closes the first. The module registers its own
`on_key` and `on_tick` the first time a list is opened, the way `neotmux.guard` arms itself.

**Behaviour that is the module's, not the caller's:**
- `j`/`k` move the cursor and wrap; the cursor is clamped to the items every tick, so a list
  whose items go away under it does not point past the end.
- When there are more items than rows, the visible window follows the cursor, the way
  `plugin.sidebar` scrolls: the cursor never leaves the screen and the window moves as little
  as it can.
- The float is centred over the **pane area** (remembered from the last tick's `ctx:pane_rect`
  union), not over the terminal, so it does not sit on the status bar or the sidebar.
- The frame is `neotmux.box`; the selected row is filled with `M.colors.cursor` before its runs
  are drawn.
- A key nothing above claims is swallowed while a list is up. A list that let unclaimed keys
  through would type into the pane behind it.

- [ ] **Step 1: Write the failing tests**

Create `crates/neotmux-lua/tests/list.rs`. Copy the harness from `crates/neotmux-lua/tests/guard.rs`
(`shipped_layer`, a temp layer in front, `begin_frame` with one pane) and drive it with
`sub.dispatch_key(support::one(), spec)` and `sub.frame_output()` the way `tests/drawing.rs`
does — read that file for how a test reads back what was drawn.

```rust
/// The list draws what the caller's `row` says, with the cursor on the first item.
#[test]
fn a_list_draws_its_items_and_its_title() { /* open a list of three, tick, assert the three
   texts and the title are among the drawn runs */ }

/// j and k move and wrap, and the caller's `done` is handed the item under the cursor.
#[test]
fn j_and_k_move_the_cursor_and_enter_chooses_what_is_under_it() { /* open, j, j, Enter,
   assert the logged choice is the third item; open again, k, Enter, assert the last */ }

/// The items are asked for again on every key, so a list whose items went away does not
/// point past the end.
#[test]
fn the_cursor_is_clamped_to_the_items_that_are_there_now() { /* items shrink between keys */ }

/// More items than rows: the window follows the cursor rather than the cursor leaving.
#[test]
fn a_list_longer_than_its_rows_scrolls_to_keep_the_cursor_on_screen() { /* twenty items,
   rows = 5, press j eight times, assert the ninth item is drawn and the first is not */ }

/// Esc closes, and a closed list draws nothing and claims no keys.
#[test]
fn esc_closes_the_list_and_gives_the_keyboard_back() { }

/// A key the caller claimed reaches it with the item under the cursor.
#[test]
fn a_key_the_caller_claimed_is_handed_the_item() { }

/// Unclaimed keys are swallowed: a list that let them through would type into the pane.
#[test]
fn a_key_nobody_claimed_is_swallowed_while_a_list_is_up() { }

/// `ask` turns the footer into an input line and hands back what was typed.
#[test]
fn ask_collects_a_line_and_hands_it_to_done() { /* open, ask, type a b c, Enter */ }

/// `filter` turns the input line on for the life of the list and narrows what is drawn.
#[test]
fn a_filtered_list_narrows_as_it_is_typed_into() { }
```

Write the assertions out in full when you implement them — the sketches above say what each
must prove, not what to type.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test list 2>&1 | tail -20`
Expected: FAIL — `module 'neotmux.list' not found`.

- [ ] **Step 3: Write the module**

`runtime/lua/neotmux/list.lua`, in the voice of `runtime/lua/neotmux/box.lua`: a header saying
what it is for and why the host has never heard of it, then the code. Read
`runtime/lua/plugin/sessions.lua` before you start — the placement, the clamping, the drawn
caret and the float resize dance are all there already, and this module is those parts with the
session-specific ones taken out. Read `runtime/lua/plugin/sidebar.lua`'s scrolling for the
window-follows-cursor rule.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p neotmux-lua --test list`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add runtime crates
git commit -m "$(cat <<'EOF'
a list is a convention, not three copies of one

The session picker and the sidebar each grew their own selection, their own
scrolling and their own row drawing, and a palette would have been the
third. `neotmux.list` is those parts with the data taken out — a float, a
cursor the keyboard moves, a window that follows it, and an input line where
one is wanted.
EOF
)"
```

---

### Task 2: The session picker is rebuilt on it

**Files:**
- Modify: `runtime/lua/plugin/sessions.lua`
- Test: `crates/neotmux-lua/tests/picker.rs` (create)

**Interfaces:**
- Consumes `neotmux.list` from Task 1.

The proof that Task 1's parts are sufficient. The picker keeps every key it has — `C-b s` to
open, `j`/`k`, `Enter`, `c`, `r`, `d`, `Esc`/`q` — and keeps what it draws.

- [ ] **Step 1: Pin what it does today, before touching it**

Write `crates/neotmux-lua/tests/picker.rs` **against the current implementation** and watch it
pass before the rebuild: open the picker over two sessions, assert the drawn runs (names, the
`p` counts, the hint line, the title), then `j`, `Enter`, and assert the switch op that comes
out. Add the `c` and `r` paths: `c`, type a name, `Enter`, assert the create and switch ops;
`r`, type, `Enter`, assert the rename. This file is the contract the rebuild has to keep.

Run: `cargo test -p neotmux-lua --test picker`
Expected: PASS against the old implementation.

- [ ] **Step 2: Rebuild the picker**

Rewrite `runtime/lua/plugin/sessions.lua` as a composition: `list.open` with `items` =
`neotmux.session.list()`, `row` = what the current tick draws for one session (the state icon,
the name, the right-hand `command  Np`), `footer` = the hint line, `keys` for `c`/`r`/`d`, and
`done` for the switch. `c` and `r` become `handle:ask{…}`. Everything the module now owns —
placement, clamping, the float dance, the caret — comes out of this file.

The file should lose most of its length. If it does not, the module is missing something: say
which in your report rather than keeping the copy.

- [ ] **Step 3: The pinning tests still pass, unchanged**

Run: `cargo test -p neotmux-lua --test picker && cargo test --workspace`
Expected: PASS, with `picker.rs` **not edited** since step 1. If a test needs changing, that is
a behaviour change — stop and say so in your report rather than changing the test.

- [ ] **Step 4: Commit**

```bash
git add runtime crates
git commit -m "$(cat <<'EOF'
the session picker is a composition now

Same keys, same rows, same frame — and a third of the file, because the
selection, the scrolling, the placement and the caret are `neotmux.list`'s.
The tests were written against the old one and not touched since, which is
what says the parts were sufficient.
EOF
)"
```

---

### Task 3: `neotmux.command`, the registry

**Files:**
- Create: `crates/neotmux-lua/src/commands.rs`
- Create: `crates/neotmux-lua/src/api/command.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`, `crates/neotmux-lua/src/api/mod.rs`
- Test: `crates/neotmux-lua/tests/commands.rs` (create), `crates/neotmux-lua/tests/subsystem.rs`

**Interfaces:**
- Produces, in Lua:

```lua
neotmux.command.add('scratch', function(args) ... end, { desc = 'open a scratch window' })
neotmux.command.list()          -- { { name =, desc =, source = }, ... }, in name order
neotmux.command.run('scratch', { 'an argument' })   -- true, or false and why
```

- Produces, in Rust: `LuaSubsystem::commands()` → the names with their descriptions and
  sources, and `LuaSubsystem::run_command(session, name, args) -> CommandOutcome`, for the
  host caller the CLI becomes later. `CommandOutcome` is `Ran(Vec<String>)`, `Failed(String)`
  or `Unknown`.

**The spec says `neotmux.command('scratch', fn)`.** It ships as `neotmux.command.add`, because
the top-level tables in this ABI are sealed and a callable table is a metatable the seal would
have to make an exception for — and because `keymap.set` is the shape this ABI already uses for
"register a named thing". Amend the spec's §4 snippet to `add` as part of this task, in its own
voice.

Registration rules, which are `keymap.set`'s and for the same reasons:
- A name already registered is refused, logged, and answered with `nil, message`; the first
  registration holds. A config layer's `init.lua` runs before the shipped one, so the user's
  name wins.
- The handler's source is recorded, so `list()` can say where a command came from.
- `run` is `Unknown` for a name nobody registered, `Failed` where the handler raised, and
  `Ran` with the lines the handler returned — `nil`, a string, or an array of strings, the way
  the spec's §4 says. Any other return is an error against the plugin.
- The handler runs under the tick budget, like every other callback, and an overrun is a
  `Failed`.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/commands.rs`, harness copied from `tests/guard.rs`:

```rust
#[test] fn a_command_can_be_added_and_listed_with_its_description() {}
#[test] fn a_command_runs_and_its_lines_come_back() {}
#[test] fn a_name_nobody_registered_is_unknown() {}
#[test] fn a_handler_that_raises_is_a_failure_naming_its_source() {}
#[test] fn a_second_registration_of_a_name_is_refused_and_the_first_holds() {}
#[test] fn the_arguments_reach_the_handler_as_an_array_of_strings() {}
#[test] fn a_command_can_be_called_from_lua_by_name() {}
#[test] fn list_answers_in_name_order() {}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test commands 2>&1 | tail -20`
Expected: FAIL — `neotmux.command` is not a field of the sealed table.

- [ ] **Step 3: Write the registry**

`commands.rs` holds a `Commands` value beside `Registry`: a `RefCell<BTreeMap<String, Entry>>`
where `Entry` is `{ callback: Rc<RegistryKey>, owner: SourceId, desc: Option<String> }`. Name
order comes free from the `BTreeMap`. Mirror `registry.rs`'s shape and borrow discipline —
never hold a borrow across a call into Lua, the way `offer_write` does not.

- [ ] **Step 4: Install the API**

`api/command.rs`, installed in `api/mod.rs` beside `keymap`, sealed as `"neotmux.command"`.
Add `command` to the ABI fixture in `tests/subsystem.rs`, in its sorted position.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux-lua`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates docs
git commit -m "$(cat <<'EOF'
a command has a name the host holds

The command surface spec's §4, in process: a plugin registers a named
callback, anything can list what is registered, and anything can call one by
name. The names live in Rust rather than in a Lua table because the CLI will
have to resolve one without entering the VM — that is the next spec's half,
and this one is what a key binding and a palette need.
EOF
)"
```

---

### Task 4: The palette

**Files:**
- Create: `runtime/lua/plugin/palette.lua`
- Modify: `runtime/lua/plugin/prefix.lua`, `runtime/init.lua`, `runtime/lua/neotmux/defaults.lua`
- Modify: `docs/plugins.md`
- Test: `crates/neotmux-lua/tests/palette.rs` (create)

**Interfaces:**
- Consumes `neotmux.list` (Task 1) and `neotmux.command` (Task 3).

Two halves:

**The prefix's verbs become commands.** `plugin.prefix` registers each of its `commands`
entries through `neotmux.command.add` with a description, and its key table calls
`neotmux.command.run`. One definition, two ways in — which is the whole argument for a registry,
and it gives the palette something to list on a fresh install. Keep the key table as the place
the key-to-name mapping lives; only the bodies move.

**The palette.** `runtime/lua/plugin/palette.lua`, on by default, bound to `:` under the
prefix:

- `list.open` with `filter` on, `items` = `neotmux.command.list()`, `row` = the name, the
  description in the quieter colour, and the source quieter still.
- Fuzzy matching, in Lua, in the file: a query matches when its characters appear in the name
  in order (`f` `o` `o` matches `f`oo`bar`). Rank by where the first character matched and how
  tightly the rest followed — a short deterministic scorer, not a library.
- `Enter` runs the command through `neotmux.command.run` and closes. Lines the command returned
  are raised with `neotmux.notify`.
- `Esc`/`q` closes.

- [ ] **Step 1: Write the failing tests**

`crates/neotmux-lua/tests/palette.rs`:

```rust
#[test] fn the_palette_lists_every_registered_command() {}
#[test] fn typing_narrows_the_list_to_what_matches_in_order() {}
#[test] fn enter_runs_the_command_under_the_cursor() {}
#[test] fn a_command_that_answers_lines_has_them_notified() {}
#[test] fn the_prefix_verbs_are_registered_as_commands() {}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p neotmux-lua --test palette 2>&1 | tail -20`
Expected: FAIL — no `plugin.palette`.

- [ ] **Step 3: Write it**

The plugin, the prefix's registrations, and the wiring: `palette` into `runtime/init.lua`'s list
and `runtime/lua/neotmux/defaults.lua` (**on** by default — it spends no screen space until the
key is pressed, which is the bargain `copy` and `sessions` are on for).

- [ ] **Step 4: Document all three**

In `docs/plugins.md`: `neotmux.command` in the ABI tables and a section of its own; the
`neotmux.list` convention beside the prompt one; the palette in the shipped-plugin table. Say
what the palette lists — commands, not keys — and that a plugin that registers a command gets a
palette entry for free.

- [ ] **Step 5: Run everything**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add runtime docs crates
git commit -m "$(cat <<'EOF'
c-b : opens every command there is

A palette is the payoff the command surface spec named: a plugin that lists
what is registered and calls one. It is a small file because the two parts
under it — the list and the registry — are not in it.

The prefix's own verbs register as commands and its keys call them by name,
so a fresh install has something to open the palette onto, and so the same
verb is one definition rather than two.
EOF
)"
```

---

## Self-Review

**Coverage:** Issue #35's `neotmux.list` half is Task 1, with Task 2 as the proof it asked for
(`plugin.sessions` rebuilt, behaviour unchanged). The command surface spec's §4 is Task 3,
in-process only. Issue #18's palette is Task 4; its operation-log half is not in this plan and
is not claimed to be.

**Deliberately not here:** `neotmux.tree` (#35's other half, for the sidebar rebuild), the style
cascade (#34), the CLI reaching plugin commands (the spec's §5/§6), and the palette asking for a
command's arguments — a command that needs arguments is listed and run with none until there is
a second consumer to design the asking against.

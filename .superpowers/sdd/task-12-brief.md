# Task 12 — the panic, the refactor, and the tests that will fight M2

M1a is merged. This is the cleanup the whole-branch review named, done before M2 starts
rather than after, because three of these items are the specific things M2 and M1b will
collide with.

## 1. A 0x0 terminal panics the renderer

Run neotmux on a PTY whose window size is never set (`pty.fork()`'s default is 0x0) and it
dies:

    vt100-0.16.2/src/grid.rs:683: attempt to subtract with overflow
    ... TermState::feed (crates/neotmux-term/src/state.rs:31) <- app::run (crates/neotmux/src/app.rs:115)

on ordinary shell output. Sizes are already clamped to at least 1 (M0a), so vt100 is being
asked to scroll a 1x1 grid and overflows. At 80x24 everything is clean, which is why no test
caught it. Terminals do report 0x0 — during startup, over ssh with no TTY size.

**Measure, do not guess.** Find the smallest `vt100::Parser` grid that survives scrolling
output — write a throwaway test that feeds enough newlines to force a scroll at 1x1, 1x2,
2x1, 2x2 and so on, and find the actual floor. Then clamp there, in the one place sizes are
clamped today, with a comment naming the vt100 version the floor was measured against. Add a
regression test at that floor and one below it.

If the floor turns out to be large enough that clamping distorts a legitimately small
terminal, say so in your report instead of clamping anyway.

## 2. `app::run` is the obstacle M2 will hit

`crates/neotmux/src/app.rs::run` is ~168 lines in one function holding, interleaved: PTY
draining, key routing, event dispatch, epoch construction, `begin_frame`, tick dispatch, op
draining (five match arms, four of which repeat the same tag lookup), the status-row geometry
flip, dirty tracking, and the draw closure — all over six mutable locals (`size`, `content`,
`panes`, `has_status`, `dirty`, `last_frame_output`). Every one of those changes when a pane
tree arrives.

Lift the frame into a struct with methods — `drain_ops`, `apply_geometry`, `render` and the
input pump as its own function — so that M2 edits method bodies instead of untangling a loop.
The six locals become fields.

This is a pure refactor: **no behaviour may change, and the test count must not drop.** If you
find a behaviour difference while moving code, that is a bug you found — report it, do not fix
it silently in the same commit.

The single-pane assumption is baked separately into three places — `pane_id()`, `pane_rects()`
and `content_size()` in the same file. Collapse them so M2 rewrites one thing, not three. Do
not build any part of a pane tree.

## 3. Four duplicated error-report blocks

`crates/neotmux-lua/src/lib.rs` repeats the same four lines — name the owner, push an error op
— in four dispatch paths. M1b's plan requires distinguishing an overrun from a runtime error
at exactly these sites; as one function that is a one-line change, as four copies it is four.
Extract `fn report(&self, owner: SourceId, err: &mlua::Error)`.

## 4. Pin the mid-tick self-removal corner, and decide the empty commit

A source that removes its own last tick registration **from inside a tick** loses the frame it
drew in that same tick: `Buffers::forget` clears scratch as well as committed, and the
per-source commit then re-inserts an empty entry via `unwrap_or_default()`.

The visible result is right (nothing drawn, nothing replayed), so this is about making it a
decision rather than an accident. Pin it with a test. Then decide, and say which you chose and
why: either `commit` skips a source whose scratch was forgotten this frame, or it keeps
re-inserting the empty entry because the result is identical and the check is not worth it.

## 5. Tests that pin structure rather than behaviour

Each of these will have to be rewritten by a refactor that changes nothing observable:

- `crates/neotmux-lua/tests/subsystem.rs::pairs_over_a_sealed_subtable_sees_its_real_fields`
  asserts a field **count** (`"3"`) for `neotmux.log`. Its sibling at the top level was already
  changed to assert the name set; do the same here.
- `crates/neotmux-lua/src/source.rs` — five unit tests drive the private `display_name`. Keep
  the one that pins the observable format; fold the rest into the integration tests that cover
  the same ground.
- `crates/neotmux-lua/src/source.rs::the_owner_is_the_first_lua_frame_above_the_host_binding`
  asserts the literal `"=[C] -> plugin/probe"`, which is a claim about mlua's stack
  representation, not about the ABI. Assert only the owner.
- `crates/neotmux/src/panes.rs` and the `pane.split('vertical') arrived` / `pane.focus(%1)
  arrived` / `nothing changed` assertions in `crates/neotmux/tests/fixtures.rs` pin the exact
  wording of placeholder log lines that exist only until M2 wires the tree. Assert that the op
  arrived attributed to `plugin/panes`, not what the placeholder says.

## 6. Minors worth doing while here

- `crates/neotmux-lua/src/lib.rs` — `dispatch_event` drops the whole event, all subscribers, if
  `shared_bytes` fails, and silently skips a subscriber whose payload table cannot be built.
  Both are unattributed silent failures in a system whose thesis is attributed failure. Log
  them against the right source.
- `crates/neotmux-lua/src/api/event.rs` — `shared_bytes` builds a Lua string from every PTY
  chunk **before** checking whether anyone subscribes to `pane_output`. With no subscribers,
  which is the default, that is a copy of the entire shell output per chunk per frame. Check
  the subscriber list first.
- `crates/neotmux-lua/src/api/ctx.rs` — `line` and `marker` accept a colours table and silently
  discard `bg`. Reject a `bg` on these rather than ignoring a field the caller supplied.
- `crates/neotmux-lua/src/api/ctx.rs` — `size()` returns `(cols, rows)` positionally while the
  `resized` event gives named `rows`/`cols` and `pane_rect` returns a named table. Three
  spellings of one question, one of them in the opposite order. Make `size()` return a named
  table `{rows = , cols = }`. This is an ABI change; do it now, before anyone depends on it,
  and update the fixtures and §4.5.
- `crates/neotmux-core/src/proto/draw.rs` — `DrawCmd::Marker` is `Text` minus `bg`, and
  `overlay::apply` renders it by calling the text path with a default background. **Check
  whether it has any distinct behaviour at all.** If it does not, delete the variant and
  `ctx:marker` with it; one way to draw text is better than two that differ in nothing. If it
  does, say what the difference is.
- `crates/neotmux-lua/src/ops.rs` — `len`/`is_empty` have no production caller. Delete them.
  Keep `Sources::interned`: it is the only way tests can observe intern order, which the
  drawing guarantee rests on.
- `crates/neotmux/src/app.rs` — `*panes.get(&pane_id()).unwrap()` cannot panic today and
  becomes a real edge the moment M2 makes the map dynamic. Handle it while restructuring.
- `crates/neotmux/src/notify.rs` — the exit transcript grows unbounded for the life of the
  session. Cap it, and say in the log when the cap drops lines.

## Constraints

- Tests pin behaviour, not structure. That is the point of item 5; do not undo it elsewhere.
- No speculative abstraction. Item 2 is a decomposition of code that exists, not a framework
  for code that does not.
- `cargo test --workspace`, `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -- -D warnings` clean, no `#[allow(...)]`.
- English, spaces, comments only where the WHY is non-obvious. No `Co-Authored-By` trailers.

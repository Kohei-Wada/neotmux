# neotmux — the crate boundaries the spec already drew

Date: 2026-09-23
Status: proposed
Scope: making the workspace's dependency graph and the placement of its code match §2 and
§2.1 of `2026-09-19-neotmux-design.md`. No feature, and no behaviour that changes: the one
case this was expected to correct turns out to be unreachable (§6). Nothing here is visible
from Lua.

## 1. Goal and non-goals

### Goal

Two pictures come out right.

**The graph.** At crate granularity the workspace differs from the design in exactly one
edge: `neotmux-lua` depends on `neotmux-term`, which §2's table forbids in as many words
("must not know about: ratatui, PTY"). Three things cross that edge and nothing else does.
Take all three away and the edge goes with them.

**The placement.** §2 gives `neotmux-core` "session / window / pane tree, layout … state
transitions". It has layout and nothing else. `PaneLayout` and the window set — which own no
PTY and are pure over `PaneId` — sit in the binary's own library instead.

### Non-goals

- **Splitting `neotmux` into client and server crates.** The cut exists in the code already —
  the server loop reaches for four of the library's modules, the client for four, and they
  share two — and this design is a prerequisite for it rather than the thing itself, because
  a client crate that still reached `neotmux-term` for its reads would not be one.
- **Moving `session`, `shells` or `reads` into `neotmux-core`.** The design says the binary
  owns the map from `PaneId` to its PTY handle and asks core for the rects. It already does.
- **Renaming `neotmux-core`.** It is a fair question once it holds what it is supposed to
  hold; it is not one to answer while it does not.
- **`PROTOCOL_VERSION`.** No message gains, loses or reshapes a field on the wire (§4).

## 2. What changes, in one paragraph

Four things move or change shape. The pane-read vocabulary — a trait and four types that
today exist twice, once for the process and once for the wire — becomes one vocabulary in
`neotmux-proto`. The clipboard stops being a write to `stdout` from inside the Lua crate and
becomes a contract the host satisfies, declared where the crate that needs it can declare it.
The column-width rule moves to the crate that owns the grid, and the two places that kept
their own copy of it start calling it. `MIN_GRID` stops being a constant `PaneLayout` reaches
for and becomes a number its caller passes, which is what §"Rounding and minimum sizes" said
it was; that is what lets `PaneLayout` and the windows over it move to `neotmux-core`.

## 3. The pane-read vocabulary is one vocabulary

### The problem

The same read is described twice.

| in process (`neotmux-term`) | on the wire (`neotmux-proto`) |
|---|---|
| `read::Text { lines, wrapped, revision }` | `msg::Lines { lines, wrapped, revision }` |
| `read::History { len, dropped }` | `msg::Extent { len, dropped }` |
| `read::Want { trailing, wrapped }` | `msg::Read::Content { pad: bool, wrapped: bool }` |

The pairs differ in `usize` against `u64` and in a two-variant enum flattened to a `bool`.
About a hundred lines of `crates/neotmux/src/reads.rs` are the conversion between them, and
nine `as` casts are how the numbers cross.

This is the thing §2.1 decided against on 2026-09-20, in its own words: *the Lua-facing types
and the wire types are the same types. One definition each, no conversion layer.* The
decision was taken about the frame and the draw commands, where it holds. It was not carried
to the reads, which arrived later.

It is also *why* the forbidden edge exists. `neotmux-lua` depends on `neotmux-term` for reads
only to name `Want`, `Text`, `History` and `Trailing`. Move the names and the reason goes.

### The shape

A `read` module in `neotmux-proto`, beside `msg`:

```rust
pub enum Trailing { Trim, Pad }
pub struct Want   { pub trailing: Trailing, pub wrapped: bool }
pub struct Lines  { pub lines: Vec<String>, pub wrapped: Option<Vec<bool>>, pub revision: u64 }
pub struct Extent { pub len: u64, pub dropped: u64 }

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

`msg::Read::Content` and `msg::Read::History` carry a `want: Want` where they carried
`pad: bool, wrapped: bool`. `Reading` is untouched: it already answers in `Lines` and
`Extent`.

**The names are the wire's.** `Lines` and `Extent` stay; `Text` and `History` go. `Text` says
less than `Lines` does, and `History` the struct sat next to `Read::History` the variant
meaning something else.

**The numbers are `u64`.** Every field `neotmux-proto` stores or sends is already fixed
width — `revision`, `len`, `dropped`, `from`, `count` are `u64`, geometry is `u16` — and
`usize` appears only as the return of a calculation (`Size::area`, the grid's index
arithmetic). Indexing with `usize` is `neotmux-term`'s own business, so `neotmux-term` casts
at its own edge rather than the protocol bending to a type whose width depends on the
machine. The Lua side casts to `i64` either way, and does one cast now where it did two.

### Where the trait lives, and why it is not the same answer as the clipboard's

One rule decides both: **a contract lives in the lowest crate that both its consumer and
every one of its implementors can see.**

`PaneRead` is consumed by `neotmux-lua` and implemented twice — by `PaneTerminals` in
`neotmux-term` and by `RemotePanes` in the binary. `neotmux-term` must not depend on
`neotmux-lua`, so the trait cannot live there. Putting it in `neotmux-lua` would also make
the server — which runs no Lua at all, by M3's decision — depend on the Lua crate to name a
trait. `neotmux-proto` is the floor both sides stand on, and the wire already carries the
same exchange as `Read` and `Reading`, so the trait ends up beside the messages that are its
other face.

### What it deletes

`reads.rs` loses its conversion. `RemotePanes` and `PaneTerminals` implement one trait over
one set of types. `neotmux-lua` loses its reason to know `neotmux-term` exists for reads.

## 4. The wire does not move

`Trailing` has two variants, so it encodes as the one byte `pad: bool` encoded as, and `Want`
is that byte followed by the `wrapped` byte — the order the two were already written in. The
bytes on the wire are therefore the bytes that were there before, and `PROTOCOL_VERSION` stays
at 3.

This is a claim about a hand-written codec, so it is a test and not a sentence: a recorded
byte sequence of each affected message, decoded, must produce the values it produced before.
If the bytes do move, the version moves with them and this section is wrong rather than the
change being wrong.

## 5. The clipboard is a contract, not a write

`crates/neotmux-lua/src/api/clipboard.rs` writes an OSC 52 sequence to `io::stdout()`. That
is not a type in the wrong crate; it is the Lua crate holding the client's terminal. Moving
`osc52` would move the import and leave the fact.

```rust
// neotmux-lua
pub trait Clipboard {
    fn set(&self, text: &str) -> std::io::Result<()>;
}
```

`LuaSubsystem::build_named` takes one more argument, an `Rc<dyn Clipboard>`. `osc52` stays in
`neotmux-term`, where an escape sequence belongs, and the client binary gives the
implementation: write the sequence to stdout and flush it.

By the rule in §3, this trait belongs in `neotmux-lua` and not in `neotmux-proto`: the only
implementor is the client binary, which already depends on `neotmux-lua`, so declaring it
where it is consumed introduces no upward edge. It is not on the wire and never will be — the
client writes to its own terminal.

**What a plugin sees does not change.** `neotmux.clipboard.set` stays synchronous and its
`true` keeps meaning exactly what `api/clipboard.rs` documents at length: the sequence was
written to the client's terminal and flushed, and nothing more. An op-based design would have
had to break that, because an op has no answer to give back inside the call.

## 6. One column rule, in one place

`neotmux-term::width` measures a string in grid columns by `vt100` 0.16.2's own rule. Its doc
says measuring any other way "would let a plugin's columns and the renderer's columns
disagree, which is the whole reason this is in the host rather than in Lua."

Three places measure, and two of them do not call it:

| where | rule |
|---|---|
| `neotmux-term/src/width.rs` | `width(c)`, and for `None`: one column at U+0100 and above, none below |
| `neotmux/src/overlay.rs:94` (`draw_text`) | `width(c).unwrap_or(0)` |
| `neotmux-lua/src/float.rs:134` (`clip_text`) | `width(c).unwrap_or(0)`, with a doc line saying it is identical to `draw_text`'s |

**They agree, and the difference between the rules is unreachable.** `unicode_width` returns
`None` for exactly 65 code points — the C0 and C1 controls and DEL — and every one of them is
below U+0100, so `of_char`'s `None => 1` arm can never be taken. Checked by enumerating every
code point rather than reasoned from the documentation.

So this is deduplication and not a fix: no character is measured differently afterwards, and
nothing a user can see changes. What it buys is that the sentence in `width.rs`'s doc becomes
structurally true instead of true by luck — the next person to write a third copy cannot
quietly pick a different rule — and that two crates stop depending on `unicode-width` to hold
their own copy of it.

`width` moves to `neotmux-proto`, which owns `GridSnapshot` and `SnapCell`: how many columns a
character takes on the grid is a property of the grid. `overlay.rs::draw_text` and
`float.rs::clip_text` call it. `of_char` keeps its unreachable arm, because it mirrors
`vt100`'s own `Screen::text` and a mirror that drops a case stops being one — with a line
saying it is unreachable at this version and why it is kept.

**The dependency moves rather than multiplies.** `unicode-width` goes from three direct
dependents (`neotmux-term`, `neotmux-lua`, `neotmux`) to one (`neotmux-proto`). §2.1's
argument for proto being a crate is that external tools should be able to depend on the
protocol alone, and a tool that only decodes messages will now also build a small, widely used
crate it does not call. That is the price of one rule for one grid, and the workspace as a
whole carries less of it than before.

## 7. `MIN_GRID` is the caller's, and that is what frees `PaneLayout`

`crates/neotmux/src/panes.rs` imports `neotmux_term::state::MIN_GRID` and uses it in four
places: `applied`, `showable`, `split` and `resize`. The design already ruled on this, in
§"Rounding and minimum sizes": **the minimum is the caller's**, passed into `Tree::split`,
and `MIN_GRID` stays in `neotmux-term` "where it is what it is: the smallest grid `vt100`
0.16.2 survives, not a property of layout". `Tree` obeys that. `PaneLayout` does not.

`PaneLayout::new` takes the minimum as a `Size`. The binary passes `MIN_GRID`. The inline
tests pass it explicitly.

That is the last thing in `panes.rs` that knows a terminal exists, so:

```
neotmux-core/src/
  layout.rs    unchanged
  panes.rs     from neotmux/src/panes.rs    (PaneLayout)
  windows.rs   from neotmux/src/windows.rs  (Surface, the window set)
```

`windows.rs` uses `neotmux-proto` and `PaneLayout` and nothing else, so it moves as it is.
`neotmux-core` keeps its single dependency on `neotmux-proto` and its property from §2 —
testable with no terminal and no Lua — which is the check on whether the boundary is real.

`session.rs` reaches for `neotmux_core::{panes, windows}` afterwards and is otherwise
unchanged.

`tui.rs` keeps calling `neotmux_term::state::clamp_to_min_grid`. The client is part of the
`neotmux` package, which is allowed to depend on everything; only `neotmux-lua`'s edge was
ever illegal. It is worth naming here because a later client/server split would have to
revisit it.

## 8. What the graph looks like afterwards

```
                    neotmux
                   ╱    │     │    ╲
                  ▼     ▼     ▼     ▼
         neotmux-lua  neotmux-core  neotmux-term
                  ╲     │           ╱
                   ▼    ▼          ▼
                    neotmux-proto
```

`neotmux-lua` keeps `mlua` and `anyhow` and drops both `neotmux-term` and `unicode-width`.
`neotmux-core` keeps `neotmux-proto` alone. `neotmux-term` keeps `portable-pty`, `vt100`,
`base64` and `anyhow`, and drops `unicode-width`. `neotmux-proto` gains `unicode-width` and
holds nothing else. `neotmux` drops its own `unicode-width`.

`neotmux-term` stays in `neotmux-lua`'s `[dev-dependencies]`. Its tests build a real
`PaneTerminals` as the host stand-in — a fake would be a re-implementation of `vt100` — and a
dev-dependency is not an edge in the shipped graph.

## 9. Order, and the thing that says it worked

| | change | needs |
|---|---|---|
| T1 | `width` to `neotmux-proto`; `overlay.rs` calls it | — |
| T2 | `Clipboard` trait in `neotmux-lua`; the client implements it | — |
| T3 | the read vocabulary unified into `neotmux-proto::read` | — |
| T4 | `MIN_GRID` becomes a `PaneLayout` parameter | — |
| T5 | `panes.rs` and `windows.rs` to `neotmux-core` | T4 |
| T6 | delete `neotmux-term` from `neotmux-lua`'s `Cargo.toml` | T1, T2, T3 |

T6 is the acceptance criterion for the whole design and it is not a judgement: the line comes
out, the workspace builds, and the edge is gone — or it does not build, and it is not.

## 10. Tests

The existing suites cover the behaviour this must not change: `socket` (25), `fixtures` (97),
the `neotmux-lua` tests, and `panes.rs`'s own inline tests, which move with it.

Three are added.

- **The wire did not move.** A recorded encoding of `Read::Content` and `Read::History`
  decodes to the same values through `Want` that it decoded to through `pad`/`wrapped`.
- **One column rule, and the arm that cannot be reached.** `of_char` agrees with
  `width(c).unwrap_or(0)` on every code point — the enumeration §6 rests on, kept as a test so
  that a later `unicode-width` which returns `None` above U+0100 is a failing test rather than
  a silent change of the rule in two of the three former call sites.
- **The clipboard reaches its sink.** The sequence for a given text arrives at a test
  implementation of `Clipboard`, which is where `api/clipboard.rs`'s existing sink test
  already points.

## 11. The spec this amends

`2026-09-19-neotmux-design.md` §2 and §2.1:

- §2's table row for `neotmux-core` becomes true for the first time.
- §2.1 gains what `neotmux-proto` holds beyond boundary data: two of the contracts the
  layers speak through, and the grid's column rule — with the dependency that costs (§6).
  The rule it records is the one in §3: a contract lives in the lowest crate that both its
  consumer and every implementor can see, which is why `PaneRead` lands in `neotmux-proto`
  and `Clipboard` does not.
- §2's table also gives `neotmux-core` the event bus and the op buffer. They are in
  `neotmux-proto` and `neotmux-lua`. That is M3's doing — the Lua layer runs on the client,
  and the ops are the plugins' — and it is right; the table is what is out of date. Noted
  there rather than changed here.

## 12. What this does not touch

The Lua ABI, the wire, `PROTOCOL_VERSION`, the frame loop, the server, the session, the
shells, every plugin, and every behaviour a user can observe. Nothing in this design changes
what anybody sees; §6 was expected to change one character class and, on checking, does not.

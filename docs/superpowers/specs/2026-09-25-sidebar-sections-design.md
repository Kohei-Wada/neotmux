# neotmux — a sidebar of sections

Date: 2026-09-25
Status: proposed
Scope: `plugin.sidebar` as a container that stacks sections a user can add, replace and
reorder; the session tree and a herdr-style agent list as the two it ships; and a session tree
that no longer loses sessions off the top when there are many panes.

## 1. Goal and non-goals

### Goal

`plugin.sidebar` is one `on_tick` that builds every row of a session tree and scrolls it so the
focused pane sits in the middle. What goes on a row is configurable (`rows`, `tokens`), but what
the sidebar *is about* is not: a user who wants something else down the left forks the file.

With ten sessions and a lot of panes in the current one, the centring pushes the sessions above
the current one off the top, with nothing saying they are there.

This spec makes three changes:

1. **Sections.** The sidebar is a stack of sections, each a plain table. It owns the band, the
   width, how the height is shared out, the separators and the fold, and nothing else.
2. **Two shipped sections.** `plugin.sidebar.sessions` is the session tree that exists today.
   `plugin.sidebar.agents` is new: every pane an agent ruleset claims, in every session, with
   its state and how long it has been in it.
3. **Sessions never scroll off first.** The session tree fits itself to its height by dropping
   the current session's subtree before it drops a session name.

### Non-goals

- **Host changes.** Everything here is Lua over APIs that exist. The one addition outside the
  sidebar is `neotmux.detect.agent(pane)` (§5.1), which is Lua too.
- **A line of what the agent said.** The second row of an agent entry — the question Claude is
  asking, read off its screen — was considered and left out: it needs a text-extraction layer
  over the detector's view that nothing else wants yet.
- **Keyboard focus in the sidebar.** The sidebar draws and does not take keys, as today. Jumping
  to an agent is what `plugin.sessions` and the palette are for.
- **Compatibility shims.** The layout fields move with the tree (§4.4); the old names are not
  kept as aliases.

## 2. The section contract

A section is a table:

```lua
{
  title = 'agents',            -- optional: a header row above it; nil for none
  share = 0.5,                 -- optional: the most of the band it takes when there is not room
                               -- for everybody, as a fraction; nil for no cap
  build = function(sidebar)    -- required
    return { rows = { runs, ... }, anchor = 3 }
  end,
  fit = function(model, height, sidebar)  -- optional
    return { runs, ... }       -- at most `height` rows
  end,
}
```

- `build` returns every row the section would draw given unlimited height, and optionally the
  index of the row that most needs to stay on screen. A row is a list of runs in the shape
  `status.segments.draw` takes. `sidebar` is the `plugin.sidebar` module, passed in so a section
  can read `sidebar.colors.bar` without requiring the module that is requiring it.
- `fit` cuts the model to a height. A section without one gets `sidebar.scroll` (§3.3).
- A section whose `build` returns no rows takes no height and draws no header.

The shipped list is:

```lua
local sidebar = require('plugin.sidebar')
sidebar.sections = {
  require('plugin.sidebar.sessions'),
  require('plugin.sidebar.agents'),
}
```

and a user changes it by assigning to it from their own `init.lua` — the same convention every
shipped plugin uses. `table.insert(sidebar.sections, mine)` adds one; `table.remove` drops one.

A section that raises in `build` or `fit` is skipped for that frame and reported once through
`neotmux.notify`, naming its `title` (or its index when it has none). The other sections still
draw.

## 3. The container

### 3.1 Width

The width is taken from the widest row across every section's model, between `widths.min` and
`widths.max`, as today. Folding still resizes to `widths.closed`.

### 3.2 Height

Let `h` be the band's height, and for each section `n` its row count plus one for its header
(when it has a title and is not the first section with rows).

- If the sum of `n` fits in `h`, each section gets `n`.
- Otherwise each section with a `share` gets `min(n, floor(h * share))`, and what is left of
  `h` goes to the sections without a `share`, in order, each taking up to its `n`. Anything
  still left goes to the capped sections in order, up to their `n`.

The shipped sessions section has no `share`; agents has `share = 0.5`. With few agents the tree
has nearly the whole band; with many agents each gets half.

A section given less than its header plus one row draws nothing.

### 3.3 `sidebar.scroll(model, height, marks)`

The default fit. When every row fits, all of them. Otherwise a window of `height` rows centred
on `anchor` (the first row when there is none) and clamped to the ends — the rule the tree
scrolls by today — with the first and last rows of the window replaced by `marks.above(n)` /
`marks.below(n)` where rows were cut on that side, `n` counting the replaced row too. A mark
never replaces the anchor. `marks` defaults to `sidebar.marks`, which draws `↑ n` / `↓ n`.

`sidebar.row(tokens, names, it)` is the other helper sections share: one row of runs from a
token table, the way the tree has always built its rows.

### 3.4 Header

A section's header is its title in the separator's colour, padded with `─` to the band's width:
`─ agents ─────────`. The first section with rows draws no header.

### 3.5 Fold

Unchanged. The rail is the first column of whatever is drawn, which for the agents section is
the state icon.

## 4. The sessions section

`runtime/lua/plugin/sidebar/sessions.lua`.

### 4.1 Rows

What the sidebar draws today: every session with its state and what it is running, and under
the current one its windows and panes as a tree.

### 4.2 Fit

In order, taking the first that fits:

1. Every row.
2. Every session row; the current session's subtree cut to what is left, scrolled around the
   focused pane (then the current window), with `… n more` in place of the cut rows on each
   side.
3. Every session at its first row only, no subtree.
4. Sessions at one row each, scrolled around the current session as in §3.3.

The ten-sessions-many-panes case is step 2: every session name stays on screen.

### 4.3 What the model carries

To fit this way the section needs more than a flat row list, so its `build` returns, beside
`rows` and `anchor`, the positions it needs: which rows are session rows, which of them are
first rows, where the subtree starts and ends, and the row of the focused pane. The shape is
the section's own business; only `rows` and `anchor` are part of the contract.

### 4.4 Configuration moves

`sessions`, `windows`, `panes` (the per-level `rows` and `row_gap`), `tokens`, `tree` and
`markers` move from `require('plugin.sidebar')` to `require('plugin.sidebar.sessions')`:

```lua
local sessions = require('plugin.sidebar.sessions')
sessions.sessions.rows = { { 'state_icon', 'name' } }
sessions.panes.rows = { { 'marker', 'tree', 'number', 'command' } }
sessions.tree.on = false
```

`widths`, `colors` (every colour, shared by the sections), `open`, `toggle` and `next_session`
stay on `plugin.sidebar`.

## 5. The agents section

`runtime/lua/plugin/sidebar/agents.lua`.

### 5.1 Which panes

Every pane, in every session (walked with `session.with`, the way the tree already walks them),
for which `neotmux.detect.agent(pane)` answers a ruleset name. `agent` is new: `detect.of`
already finds the ruleset whose rule matched and then drops its name; it is kept on the cache
entry and handed back, nil where no ruleset matched. It reads the same cache as `of`, so it
costs nothing after `state.of` has run.

### 5.2 A row

```
◉ work:2.1 claude  3m
```

Tokens `state_icon`, `location` (`session:window.pane`, numbered as the tree numbers them),
`command`, and `age`, in `rows`/`tokens` tables of the same shape as the sessions section.

### 5.3 Order

`blocked`, then `working`, then `idle`; within a state, session order, then window, then pane.
The anchor is the focused pane when it is an agent.

### 5.4 Age

The section keeps `{ state, at }` per pane and moves `at` to `neotmux.now()` only when the state
it reads differs from the one it kept. A pane seen for the first time starts at the moment it
was seen, so an age is "at least this long". An entry is dropped by the first build that no
longer finds its pane among the agents, which covers a pane closed in a session this tick is
not for as well as one closed here.

Shown as `12s` under a minute, `3m` under an hour, `2h` beyond.

### 5.5 Nothing to show

No agents, no rows: the section takes no height and draws no header, and the tree has the band.

## 6. Testing

- **`crates/neotmux-lua/tests/sidebar.rs`**, host-less, in the shape of `watch.rs`:
  - height sharing: all fits; a capped section at its share; leftovers to uncapped then capped;
  - the sessions fit steps 1–4, including ten sessions with more panes than rows keeping every
    session name, and the `… n more` rows;
  - `sidebar.scroll`'s `↑ n` / `↓ n`;
  - agents: a fake ruleset installed from `init.lua` into `neotmux.detect.agents`, listing only
    the panes it claims, across sessions, in state order, with the age moving only on a change;
  - a user section added from `init.lua` is drawn, and a section that raises is skipped while
    the others draw.
- **`crates/neotmux/tests/fixtures.rs`**: the four sidebar tests keep passing, updated for the
  moved fields.
- **By eye**: a build in the worktree with ten sessions and many panes, and with a real `claude`
  running, screens captured and read.

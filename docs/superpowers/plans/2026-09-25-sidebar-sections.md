# A sidebar of sections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `plugin.sidebar` becomes a stack of injectable sections; it ships the session tree (which stops losing sessions off the top) and a herdr-style agent list under it.

**Architecture:** Pure Lua in `runtime/`. `plugin/sidebar.lua` keeps the band, width, height sharing, headers, fold and error isolation; `plugin/sidebar/sessions.lua` and `plugin/sidebar/agents.lua` are sections (plain tables with `build`/`fit`). `neotmux.detect` gains `agent(pane)` so the agent list knows which panes a ruleset answered for.

**Tech Stack:** Lua 5.4 (mlua), Rust integration tests in `crates/neotmux-lua/tests` (host-less) and `crates/neotmux/tests/fixtures.rs` (end to end).

**Spec:** `docs/superpowers/specs/2026-09-25-sidebar-sections-design.md`

## Global Constraints

- No host (Rust `src/`) changes. Tests are the only Rust edited.
- No compatibility aliases for the fields that move to `plugin.sidebar.sessions`.
- `widths.min` 27, `widths.max` 36, `widths.closed` 1 — unchanged.
- Agents section: `title = 'agents'`, `share = 0.5`; sessions section: no title, no share.
- Age format: `12s` under a minute, `3m` under an hour, `2h` beyond.
- Order in the agent list: `blocked`, `working`, `idle`; then session, window, pane order.
- Work happens in `.worktrees/sidebar-sections` (branch `sidebar-sections`); never touch the main checkout.
- `make check` (fmt, clippy `-D warnings`, `cargo test --workspace`) passes at the end of every task.
- Commit messages follow the repository's style: a lowercase sentence saying what is now true, then the attribution trailer:
  ```
  Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
  ```

## Review Focus

1. A section row wider than `widths.max` — the band stops at 36 columns and the row is clipped, not the pane area. (Task 1)
2. A terminal too short for a header plus a row — nothing drawn outside the band, no error raised, the tree keeps its one row. (Task 4)
3. More agents than half the band — the agent list stops at its share with `↓ n`, and the tree still shows its session names. (Task 4)
4. The sidebar folded with agents listed — the rail stays one column wide. (Task 4)
5. An agent that quits (screen no longer matches) and starts again — it leaves the list, and its age starts over when it comes back. (Task 4)

---

## File Structure

- `runtime/lua/plugin/sidebar.lua` — **rewrite**: the container. Band, width, `row`, `scroll`, `marks`, height sharing, headers, error isolation, fold, `sections`.
- `runtime/lua/plugin/sidebar/sessions.lua` — **create**: the session tree section (moved out of `sidebar.lua`) and its `fit`.
- `runtime/lua/plugin/sidebar/agents.lua` — **create**: the agent list section.
- `runtime/lua/neotmux/detect.lua` — **modify**: keep the answering ruleset's name; add `agent(pane)`.
- `crates/neotmux-lua/tests/sidebar.rs` — **create**: host-less tests for all of the above.
- `crates/neotmux-lua/tests/detect.rs` — **modify**: a test for `detect.agent`.
- `docs/plugins.md`, `CHANGELOG.md` — **modify**.

---

### Task 1: the sidebar is a container, and the tree is its first section

Behaviour on screen does not change: one shipped section, scrolled exactly as today (centred on the focused row, no marks yet).

**Files:**
- Rewrite: `runtime/lua/plugin/sidebar.lua`
- Create: `runtime/lua/plugin/sidebar/sessions.lua`
- Create: `crates/neotmux-lua/tests/sidebar.rs`
- Modify: `docs/plugins.md:1378-1394` and `docs/plugins.md:1624-1642`

**Interfaces:**
- Produces (on `require('plugin.sidebar')`): `widths`, `colors` (adds `rule`), `band`, `open`, `sections` (list), `row(tokens, names, it) -> runs`, `scroll(model, height) -> rows`, `toggle()`, `next_session()`.
- Produces (on `require('plugin.sidebar.sessions')`): `markers`, `tree`, `sessions`, `windows`, `panes`, `tokens`, `build(sidebar) -> { rows, anchor, entries = { { rows, here }, ... }, tree = { rows, anchor } }`.
- Section contract: `{ title?, share?, build(sidebar), fit?(model, height, sidebar) }`.

- [ ] **Step 1: Write the host-less harness and the failing tests**

Create `crates/neotmux-lua/tests/sidebar.rs`:

```rust
//! `plugin.sidebar` and its sections, driven without a host: sessions named and framed the way
//! the server frames them, a tick for the one being looked at, and the band read back off the
//! draw commands as lines of text.

use std::collections::BTreeMap;

use neotmux_proto::{
    DrawCmd, Epoch, Geometry, PaneId, PaneInfo, Rect, SessionId, Size, Window, WindowId,
};

mod support;

use support::Built;

const SURFACE: Size = Size { rows: 12, cols: 80 };

/// One session as the host would publish it: its windows left to right, each a list of pane
/// ids, and the pane with focus. Pane ids are unique across sessions, as the server's are.
struct Sess {
    id: u32,
    name: &'static str,
    windows: Vec<Vec<u32>>,
    focus: u32,
    /// Panes that report `agent` as their command; every other pane reports `cat`.
    agents: Vec<u32>,
}

impl Sess {
    fn new(id: u32, name: &'static str, windows: Vec<Vec<u32>>) -> Self {
        let focus = windows[0][0];
        Self {
            id,
            name,
            windows,
            focus,
            agents: Vec::new(),
        }
    }

    fn focused(mut self, pane: u32) -> Self {
        self.focus = pane;
        self
    }

    fn agents(mut self, panes: &[u32]) -> Self {
        self.agents = panes.to_vec();
        self
    }
}

/// The shipped runtime with only the sidebar on, and no agent rulesets: a test about the tree
/// must not depend on whether the shipped Claude rules happen to match a blank `cat` pane. The
/// agent tests install a ruleset of their own in `extra`.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    support::built(&format!(
        "require('neotmux.defaults').sidebar = true\n\
         require('neotmux.detect').agents = {{}}\n\
         {extra}"
    ))
}

fn open(sub: &Built, sessions: &[Sess]) {
    for s in sessions {
        for pane in s.windows.iter().flatten() {
            sub.terminals().open(PaneId::from(*pane), SURFACE, 0);
        }
    }
}

fn frame_in(sub: &Built, surface: Size, sessions: &[Sess], current: u32) -> Vec<String> {
    let reserved = sub.reserved();
    let content = Size {
        rows: surface.rows - reserved.rows(),
        cols: surface.cols - reserved.cols(),
    };
    let area = Rect::new(reserved.left, reserved.top, content.cols, content.rows);
    for s in sessions {
        let id = SessionId::from(s.id);
        sub.name_session(id, s.name);
        sub.lay_out_reservations(id, surface, reserved);
        let focus = PaneId::from(s.focus);
        let at = s
            .windows
            .iter()
            .position(|w| w.contains(&s.focus))
            .expect("focus is in one of the windows");
        let shown: Vec<PaneId> = s.windows[at].iter().map(|p| PaneId::from(*p)).collect();
        let windows = s
            .windows
            .iter()
            .enumerate()
            .map(|(n, panes)| Window {
                id: WindowId::from(s.id * 100 + n as u32),
                panes: panes.iter().map(|p| PaneId::from(*p)).collect(),
                name: None,
            })
            .collect();
        sub.begin_frame(
            id,
            Epoch::new(content, shown.clone(), Some(focus)),
            Geometry {
                area,
                panes: shown.iter().map(|p| (*p, area)).collect(),
                focus: Some(focus),
                windows,
                window: Some(WindowId::from(s.id * 100 + at as u32)),
            },
        );
        let info: BTreeMap<PaneId, PaneInfo> = s
            .windows
            .iter()
            .flatten()
            .map(|p| {
                let command = if s.agents.contains(p) { "agent" } else { "cat" };
                (
                    PaneId::from(*p),
                    PaneInfo {
                        command: Some(command.to_string()),
                        cwd: None,
                    },
                )
            })
            .collect();
        sub.publish_pane_info(id, info);
    }
    sub.dispatch_tick(SessionId::from(current));
    band(sub, surface)
}

fn frame(sub: &Built, sessions: &[Sess], current: u32) -> Vec<String> {
    frame_in(sub, SURFACE, sessions, current)
}

/// Two frames: the first asks for the width the rows want, and `resize_to` lands on the next.
fn settled(sub: &Built, sessions: &[Sess], current: u32) -> Vec<String> {
    frame(sub, sessions, current);
    frame(sub, sessions, current)
}

/// Every `Text` command placed at its column, one `char` per column (true of everything the
/// sidebar draws), each line's trailing blanks trimmed.
fn band(sub: &Built, surface: Size) -> Vec<String> {
    let mut grid = vec![vec![' '; surface.cols as usize]; surface.rows as usize];
    for cmd in sub.frame_output() {
        if let DrawCmd::Text { x, y, text, .. } = cmd {
            for (i, ch) in text.chars().enumerate() {
                if let Some(cell) = grid
                    .get_mut(y as usize)
                    .and_then(|row| row.get_mut(x as usize + i))
                {
                    *cell = ch;
                }
            }
        }
    }
    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
        .collect()
}

fn line_with(lines: &[String], needle: &str) -> usize {
    lines
        .iter()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?}:\n{}", lines.join("\n")))
}

#[test]
fn the_tree_is_what_the_shipped_sidebar_draws() {
    let (_dir, sub) = built("");
    let s = [Sess::new(1, "work", vec![vec![101, 102]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "work");
    line_with(&lines, "1.1 cat");
    line_with(&lines, "1.2 cat");
}

#[test]
fn a_section_added_from_init_lua_draws_under_its_title() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.sidebar').sections, {\n\
           title = 'mine',\n\
           build = function() return { rows = { { { text = 'hello' } } } } end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let tree = line_with(&lines, "work");
    let header = line_with(&lines, "\u{2500} mine \u{2500}");
    assert!(tree < header, "{lines:#?}");
    assert_eq!(lines[header + 1], "hello", "{lines:#?}");
}

#[test]
fn a_section_with_no_rows_takes_no_room_and_draws_no_title() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.sidebar').sections, {\n\
           title = 'mine',\n\
           build = function() return { rows = {} } end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert!(!lines.iter().any(|l| l.contains("mine")), "{lines:#?}");
}

#[test]
fn a_section_that_raises_is_skipped_and_named_once() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.sidebar').sections, 1, {\n\
           title = 'broken',\n\
           build = function() error('boom') end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    settled(&sub, &s, 1);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "work");
    let said: Vec<String> = support::notified(&sub)
        .into_iter()
        .filter(|t| t.contains("section 'broken'"))
        .collect();
    assert_eq!(said.len(), 1, "{said:?}");
}

/// 12 rows, wants of 3 and 31: the capped section is held to 6, the uncapped one takes its 3,
/// and the 3 left over go back to the capped one — a header and eight rows.
#[test]
fn a_capped_section_takes_its_share_and_then_what_is_left() {
    let (_dir, sub) = built(
        "local sidebar = require('plugin.sidebar')\n\
         local function rows(prefix, n)\n\
           local out = {}\n\
           for i = 1, n do out[i] = { { text = prefix .. i } } end\n\
           return out\n\
         end\n\
         local function first(model, height)\n\
           local out = {}\n\
           for i = 1, height do out[i] = model.rows[i] end\n\
           return out\n\
         end\n\
         sidebar.sections = {\n\
           { build = function() return { rows = rows('a', 3) } end },\n\
           { title = 'b', share = 0.5, fit = first,\n\
             build = function() return { rows = rows('b', 30) } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert_eq!(&lines[0..3], &["a1", "a2", "a3"], "{lines:#?}");
    assert!(lines[3].starts_with("\u{2500} b \u{2500}"), "{lines:#?}");
    let b: Vec<String> = (1..=8).map(|i| format!("b{i}")).collect();
    assert_eq!(&lines[4..12], b.as_slice(), "{lines:#?}");
}

#[test]
fn a_share_is_not_applied_when_everything_fits() {
    let (_dir, sub) = built(
        "local sidebar = require('plugin.sidebar')\n\
         local function rows(prefix, n)\n\
           local out = {}\n\
           for i = 1, n do out[i] = { { text = prefix .. i } } end\n\
           return out\n\
         end\n\
         sidebar.sections = {\n\
           { build = function() return { rows = rows('a', 3) } end },\n\
           { title = 'b', share = 0.5, build = function() return { rows = rows('b', 8) } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let b: Vec<String> = (1..=8).map(|i| format!("b{i}")).collect();
    assert_eq!(&lines[4..12], b.as_slice(), "{lines:#?}");
}

/// Review focus 1: a row wider than the widest band is cut at the band, not drawn into the panes.
#[test]
fn a_row_wider_than_the_widest_band_is_cut_at_it() {
    let (_dir, sub) = built(
        "require('plugin.sidebar').sections = {\n\
           { build = function() return { rows = { { { text = string.rep('x', 100) } } } } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert_eq!(lines[0], "x".repeat(36), "{lines:#?}");
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p neotmux-lua --test sidebar`
Expected: `the_tree_is_what_the_shipped_sidebar_draws` passes (today's sidebar draws the tree); the others FAIL — `sections` is nil (`attempt to index a nil value`) so `run_init` returns errors and `support::built` panics on its `assert!(sub.run_init().is_empty())`.

- [ ] **Step 3: Create the sessions section**

Create `runtime/lua/plugin/sidebar/sessions.lua`:

```lua
-- The session tree, the first section of `plugin.sidebar`: every session with what it is doing,
-- then the windows and panes of the one you are in.
--
-- The layout is a table, not a shape baked into this file. Each level draws one entry as a list
-- of rows, and each row is a list of tokens:
--
--   local sessions = require('plugin.sidebar.sessions')
--   sessions.sessions.rows = { { 'state_icon', 'name' }, { 'command', 'panes' } }
--   sessions.panes.rows = { { 'state_icon', 'number', 'command' } }
--
-- so moving what is on which line, or dropping a token you do not care about, is an assignment
-- rather than a fork of this file. The colours are the sidebar's, `require('plugin.sidebar').colors`.
local state = require("neotmux.state")

local M = {}

-- The sidebar's colours, set by `build` for the tokens, which are called with nothing but the
-- row they are about.
local colors

M.markers = {
  current_window = "\u{25b8}", -- ▸
  window = " ",
  focused_pane = "\u{2022}", -- •
  pane = " ",
}

-- A session holds windows and a window holds panes, so the tree draws what it is. The guides go
-- after the first column and never in it — that column is the whole of the band when it is
-- folded, and a fold showing a run of `│` says nothing about where you are.
--
--   require('plugin.sidebar.sessions').tree.on = false   -- indent without the lines
M.tree = {
  on = true,
  -- Two columns each, so a level always costs the same and the labels line up whichever
  -- branch they are on.
  glyphs = {
    branch = "\u{251c}\u{2500}", -- ├─
    last = "\u{2514}\u{2500}", -- └─
    through = "\u{2502} ", -- │
    blank = "  ",
  },
}

-- The guide in front of a row, given where it sits: one segment per level above it saying
-- whether that level had anything after it, and one for the row itself.
local function guide(levels)
  if not M.tree.on then
    return string.rep("  ", #levels)
  end
  local out = {}
  for at, last in ipairs(levels) do
    if at == #levels then
      out[#out + 1] = last and M.tree.glyphs.last or M.tree.glyphs.branch
    else
      out[#out + 1] = last and M.tree.glyphs.blank or M.tree.glyphs.through
    end
  end
  return table.concat(out)
end

-- Each level's layout: the rows of one entry, and the blank rows between entries. A row that
-- comes out empty is dropped rather than drawn, so a second row of detail costs nothing on a
-- session with no detail to give.
M.sessions = {
  rows = { { "state_icon", "name" }, { "command", "panes" } },
  row_gap = 0,
}
M.windows = {
  rows = { { "marker", "tree", "number", "name" } },
  row_gap = 0,
}
M.panes = {
  rows = { { "marker", "tree", "number", "command" } },
  row_gap = 0,
}

-- What a token is worth, given what the row is about. Add one and it is available to every
-- layout at that level:
--
--   require('plugin.sidebar.sessions').tokens.sessions.age = function(it) ... end
M.tokens = {
  sessions = {
    state_icon = function(it)
      return state.icon(it.state), colors[it.state]
    end,
    name = function(it)
      return it.name, it.here and colors.current_session or colors.session
    end,
    command = function(it)
      return it.command, colors.detail
    end,
    panes = function(it)
      return it.panes .. "p", colors.detail
    end,
  },
  windows = {
    tree = function(it)
      return it.guide, colors.detail
    end,
    marker = function(it)
      return it.current and M.markers.current_window or M.markers.window,
        it.current and colors.current_window or colors.window
    end,
    number = function(it)
      return tostring(it.number), it.current and colors.current_window or colors.window
    end,
    -- Nothing for a window nobody has named: its number is what it is called until then.
    name = function(it)
      return it.name, it.current and colors.current_window or colors.window
    end,
  },
  panes = {
    tree = function(it)
      return it.guide, colors.detail
    end,
    marker = function(it)
      return it.focused and M.markers.focused_pane or M.markers.pane,
        it.focused and colors.focused_pane or colors.pane
    end,
    state_icon = function(it)
      return state.icon(it.state), colors[it.state]
    end,
    number = function(it)
      return it.number, it.focused and colors.focused_pane or colors.pane
    end,
    command = function(it)
      return it.command, colors.detail
    end,
  },
}

-- What a pane is running, or nil where the host has not got it — a child that exited between
-- the sample and the tick, or a pane that closed while the rows were being built.
local UNKNOWN = "\u{2014}" -- em dash

local function command_of(pane)
  local info = neotmux.pane.info(pane)
  return info and info.command
end

-- What a session is doing, read through `session.with`: how many panes it has, what the one
-- somebody is looking at is running, and whether any of them is waiting for you.
local function about(name)
  local panes, command, worst = 0, nil, "idle"
  neotmux.session.with(name, function()
    local list = neotmux.pane.list()
    panes = #list
    local focus = neotmux.pane.current()
    command = focus and command_of(focus)
    worst = state.worst(list)
  end)
  return { panes = panes, command = command or UNKNOWN, state = worst }
end

-- Every row of one entry, indented past the first column for all but the first: the first
-- column is the whole of the sidebar when it is folded, so what is in it has to be the marker
-- and the rest has to start after it.
local function entry(sidebar, level, kind, it, out)
  for at, names in ipairs(level.rows) do
    local runs = sidebar.row(M.tokens[kind], names, it)
    if #runs > 0 then
      if at > 1 then
        table.insert(runs, 1, { text = "  ", bg = colors.bar })
      end
      out[#out + 1] = runs
    end
  end
  for _ = 1, level.row_gap or 0 do
    out[#out + 1] = { { text = "", bg = colors.bar } }
  end
end

-- The windows and panes of the session that is open, into `out`. Answers the row worth keeping
-- on screen: the pane you are in, or failing that the window, or nil when there is neither.
local function subtree(sidebar, out)
  local anchor
  local current_window = neotmux.window.current()
  local focused = neotmux.pane.current()
  local windows = neotmux.window.list()
  for window_at, window in ipairs(windows) do
    local current = window == current_window
    local last_window = window_at == #windows
    if current then
      anchor = #out + 1
    end
    entry(sidebar, M.windows, "windows", {
      number = window_at,
      name = neotmux.window.name(window),
      current = current,
      guide = guide({ last_window }),
    }, out)
    -- Nil for a window that closed between the list and now, which is not an error worth a
    -- sidebar with a hole in it.
    local panes = neotmux.window.panes(window) or {}
    for pane_at, pane in ipairs(panes) do
      local is_focused = pane == focused
      if is_focused then
        anchor = #out + 1
      end
      entry(sidebar, M.panes, "panes", {
        number = window_at .. "." .. pane_at,
        focused = is_focused,
        command = command_of(pane) or UNKNOWN,
        state = state.of(pane) or "idle",
        guide = guide({ last_window, pane_at == #panes }),
      }, out)
    end
  end
  return anchor
end

-- Beside the rows, what a fit needs to cut them sensibly: each session's own rows and which one
-- is here, and the subtree hung under it with the row inside it worth keeping.
function M.build(sidebar)
  colors = sidebar.colors
  local rows, entries, anchor = {}, {}, 1
  local tree = { rows = {} }
  local here = neotmux.session.current()
  for _, name in ipairs(neotmux.session.list()) do
    local it = about(name)
    it.name = name
    it.here = name == here
    local own = {}
    entry(sidebar, M.sessions, "sessions", it, own)
    entries[#entries + 1] = { rows = own, here = it.here }
    if it.here then
      anchor = #rows + 1
    end
    for _, runs in ipairs(own) do
      rows[#rows + 1] = runs
    end
    -- Under the one you are in, because that is where its windows are. The others are named
    -- and no more: what is inside a session you are not in is what the picker is for.
    if it.here then
      tree.anchor = subtree(sidebar, tree.rows)
      if tree.anchor then
        anchor = #rows + tree.anchor
      end
      for _, runs in ipairs(tree.rows) do
        rows[#rows + 1] = runs
      end
    end
  end
  return { rows = rows, anchor = anchor, entries = entries, tree = tree }
end

return M
```

- [ ] **Step 4: Rewrite the container**

Replace the whole of `runtime/lua/plugin/sidebar.lua` with:

```lua
-- A sidebar down the left: a stack of sections, each a table saying what rows it has and how to
-- cut them to a height. The shipped one is the session tree, `plugin.sidebar.sessions`.
-- `C-b b` folds the band to its first column and back, when the prefix is loaded.
--
-- Off by default, unlike the status bar: a status line spends a row nobody was using for
-- anything else, while a sidebar is an opinion about screen real estate. Turn it on from your
-- own `init.lua`:
--
--   require('neotmux.defaults').sidebar = true
--
-- What is in it is `sections`, top to bottom, a list you assign to like any other field here:
--
--   table.insert(require('plugin.sidebar').sections, {
--     title = 'mine',   -- a header above it; nil for none
--     share = 0.3,      -- the most of the band it takes when there is not room for everybody
--     build = function(sidebar) return { rows = { runs, ... }, anchor = 1 } end,
--     fit = function(model, height, sidebar) return { runs, ... } end,   -- optional
--   })
--
-- A row is a list of runs, the shape `status.segments.draw` takes. `build` is handed this module
-- rather than requiring it, because the shipped sections are required from here while this file
-- is still loading. Without a `fit`, a section is cut by `sidebar.scroll`.
--
-- Nothing here is a host feature. `neotmux.reserve('left', n)` takes the band,
-- `handle:resize_to(n)` widens and folds it, and `ctx:reserved()` answers where it landed.
local segments = require("status.segments")

local M = {}

-- What the band is worth, in columns. The width is taken from what there is to show, between
-- these two, so a session called `feature/long-branch-name` is not cut to nothing — and `min`
-- is what it settles at when there is nothing long to show, which is most of the time.
M.widths = {
  min = 27,
  max = 36,
  -- What is left when it is folded: one column, so the markers still say how many sessions and
  -- panes there are and which of them you are in. A band of nothing would be a band you have to
  -- remember is there.
  closed = 1,
}

-- Catppuccin Mocha, the palette the status bar was drawn from. Every section reads these, and
-- every field is read on the tick, so assigning to one from your own `init.lua` takes effect.
M.colors = {
  bar = "#181825", -- mantle: the band everything else sits on
  session = "#9399b2", -- overlay2: a session you are not in
  current_session = "#f9e2af", -- yellow, so a session reads as a different kind of row
  window = "#cdd6f4",
  current_window = "#cba6f7", -- mauve, the status bar's accent for the window being shown
  pane = "#6c7086", -- overlay0: a pane in a window you are not looking at
  focused_pane = "#a6e3a1", -- green
  -- The second row of an entry, and anything else that is detail rather than name.
  detail = "#585b70", -- surface2
  rule = "#45475a", -- surface1: a section's header
  -- One per state, so the icon carries its meaning in the colour as well as the shape.
  working = "#a6e3a1", -- green
  blocked = "#f9e2af", -- yellow: the one that wants you
  idle = "#585b70", -- surface2
}

M.band = neotmux.reserve("left", M.widths.min)

-- Which of the two the band is in. The rows are the same either way: `segments.draw` cuts a run
-- that does not fit, so the rail is the open sidebar with everything but the first column
-- clipped off, and there is no second way of drawing this to keep in step with the first.
M.open = true

-- One row: every token in `names` that has something to say, in order, separated by a space.
-- A token that answers nil or an empty string is left out, so a row whose tokens are all empty
-- is no row.
function M.row(tokens, names, it)
  local runs = {}
  for _, name in ipairs(names) do
    local token = tokens[name]
    if token then
      local text, colour = token(it)
      if text and text ~= "" then
        if #runs > 0 then
          runs[#runs + 1] = { text = " ", bg = M.colors.bar }
        end
        runs[#runs + 1] = { text = text, fg = colour or M.colors.detail, bg = M.colors.bar }
      end
    end
  end
  return runs
end

-- The default fit: every row if they fit, and otherwise a window of them centred on `anchor`
-- and clamped to the ends, so the row that matters is always one of the ones that stayed.
function M.scroll(model, height)
  local rows = model.rows
  if height <= 0 then
    return {}
  end
  if #rows <= height then
    return rows
  end
  local anchor = model.anchor or 1
  local first = math.min(math.max(anchor - math.floor(height / 2), 1), #rows - height + 1)
  local out = {}
  for at = first, first + height - 1 do
    out[#out + 1] = rows[at]
  end
  return out
end

local function default_fit(model, height)
  return M.scroll(model, height)
end

-- One broken section must not take the sidebar down with it, and must say which one it was —
-- once, because it will raise again on every frame after this one.
local complained = setmetatable({}, { __mode = "k" })

local function guarded(section, at, what, fn, ...)
  local ok, result = pcall(fn, ...)
  if ok then
    return result
  end
  if not complained[section] then
    complained[section] = true
    local which = "plugin.sidebar: section '" .. tostring(section.title or at) .. "' " .. what
    neotmux.notify("error", which .. ": " .. tostring(result))
  end
  return nil
end

-- How many rows each live section gets. All of what it wants when everybody fits; otherwise a
-- capped section is held to its share first, the uncapped ones take what they want of the rest
-- in order, and whatever is still left goes back to the capped ones in order.
local function share_out(live, height)
  local wants, total = {}, 0
  for i, it in ipairs(live) do
    wants[i] = #it.model.rows + (it.titled and 1 or 0)
    total = total + wants[i]
  end
  if total <= height then
    return wants
  end
  local got, left = {}, height
  for i, it in ipairs(live) do
    if it.section.share then
      got[i] = math.min(wants[i], math.floor(height * it.section.share), left)
      left = left - got[i]
    end
  end
  for i, it in ipairs(live) do
    if not it.section.share then
      got[i] = math.min(wants[i], left)
      left = left - got[i]
    end
  end
  for i, it in ipairs(live) do
    if it.section.share then
      local more = math.min(wants[i] - got[i], left)
      got[i] = got[i] + more
      left = left - more
    end
  end
  return got
end

local function header(title, width)
  local text = "\u{2500} " .. title .. " "
  local rest = width - segments.columns(text)
  if rest > 0 then
    text = text .. string.rep("\u{2500}", rest)
  end
  return { { text = text, fg = M.colors.rule, bg = M.colors.bar } }
end

-- How wide the band wants to be: the widest row any section has, between the two bounds.
local function width_for(live)
  local widest = 0
  for _, it in ipairs(live) do
    for _, runs in ipairs(it.model.rows) do
      widest = math.max(widest, segments.width(runs))
    end
  end
  return math.max(M.widths.min, math.min(M.widths.max, widest + 1))
end

-- Swap the two. A resize keeps the band's place on the left edge, which a remove and a fresh
-- reservation would not: folding would walk the sidebar inside anybody who reserved after it.
function M.toggle()
  -- Nil if the left edge was already spoken for when this file loaded.
  if not M.band then
    return
  end
  M.open = not M.open
end

-- The next session round, in the order `session.list` gives, which is name order. Not bound to
-- anything: `plugin.sessions` has `C-b s` and shows you what you are choosing between. This is
-- here for an `init.lua` that would rather step than pick.
function M.next_session()
  local names = neotmux.session.list()
  if #names < 2 then
    return
  end
  local current = neotmux.session.current()
  for at, name in ipairs(names) do
    if name == current then
      neotmux.session.switch(names[at % #names + 1])
      return
    end
  end
end

-- The prefix is the standard plugin and this one is optional, so this is the direction the
-- knowledge goes in: bound only if the prefix is already loaded, which it is by the time the
-- sidebar loads unless it was turned off.
if package.loaded["plugin.prefix"] then
  require("plugin.prefix").commands["b"] = M.toggle
end

neotmux.on_tick(function(ctx)
  local band = ctx:reserved()
  -- Nil while the terminal is too narrow to spare the columns. The panes have them back, so
  -- there is nowhere to draw and nothing to draw over.
  if not band then
    return
  end

  -- A section with nothing to show takes no room and draws no header. The first section that
  -- does show something draws no header either: the top of the band says where it starts.
  local live = {}
  for at, section in ipairs(M.sections) do
    local model = guarded(section, at, "build", section.build, M)
    if type(model) == "table" and type(model.rows) == "table" and #model.rows > 0 then
      live[#live + 1] = {
        section = section,
        at = at,
        model = model,
        titled = #live > 0 and section.title ~= nil,
      }
    end
  end

  -- The width the content wants, asked for once it is known. `resize_to` lands on the next
  -- frame, which is why this is a comparison and not a call every tick.
  if M.band then
    local want = M.open and width_for(live) or M.widths.closed
    if want ~= band.w then
      M.band:resize_to(want)
    end
  end

  ctx:rect(nil, band.x, band.y, band.w, band.h, { bg = M.colors.bar })

  local heights = share_out(live, band.h)
  local y = band.y
  for i, it in ipairs(live) do
    local body = heights[i] - (it.titled and 1 or 0)
    if body >= 1 then
      local fit = it.section.fit or default_fit
      local rows = guarded(it.section, it.at, "fit", fit, it.model, body, M)
      if type(rows) == "table" then
        local top = y
        if it.titled then
          segments.draw(ctx, band.x, top, band.w, header(it.section.title, band.w))
          top = top + 1
        end
        for k = 1, math.min(#rows, body) do
          segments.draw(ctx, band.x, top + k - 1, band.w, rows[k])
        end
      end
    end
    y = y + heights[i]
  end
end)

-- Last, so a section's own file can already find everything above in the module it is handed.
M.sections = {
  require("plugin.sidebar.sessions"),
}

return M
```

- [ ] **Step 5: Run the host-less tests**

Run: `cargo test -p neotmux-lua --test sidebar`
Expected: all 7 PASS.

- [ ] **Step 6: Run the end-to-end sidebar tests, unchanged**

Run: `cargo test -p neotmux --test fixtures sidebar`
Expected: all 4 `the_shipped_sidebar_*` PASS with no edits — this task changes no behaviour on screen.

- [ ] **Step 7: Update the docs for the moved fields**

In `docs/plugins.md`, replace the block from `The sidebar is a tree: every session with what it is running, and under the one you are in, its` through `are.` (currently lines 1378–1394) with:

````markdown
The sidebar is a stack of sections, and the shipped one is a tree: every session with what it is
running, and under the one you are in, its windows and their panes. Its width is taken from what
there is to show, between `require('plugin.sidebar').widths.min` (27) and `.max` (36). What is on
which row of the tree is a table rather than a shape baked into the file — each level draws one
entry as a list of rows, and each row is a list of tokens:

```lua
local sessions = require('plugin.sidebar.sessions')
sessions.sessions.rows = { { 'state_icon', 'name' }, { 'command', 'panes' } }
sessions.panes.rows = { { 'marker', 'tree', 'number', 'command' } }
sessions.tree.on = false          -- indent without the lines
sessions.tokens.sessions.age = function(it) ... end   -- a token of your own
```

so moving what is where, or dropping what you do not care about, is an assignment rather than a
fork of the file. The first column is left to the markers on purpose: it is the whole of the
band when the sidebar is folded, and a fold showing a run of `│` says nothing about where you
are.

A section is a table, and `require('plugin.sidebar').sections` is the list of them, top to
bottom:

```lua
table.insert(require('plugin.sidebar').sections, {
  title = 'mine',     -- a header above it; nil for none
  share = 0.3,        -- the most of the band it takes when there is not room for everybody
  build = function(sidebar) return { rows = { { { text = 'hello' } } }, anchor = 1 } end,
  fit = function(model, height, sidebar) ... end,   -- optional: cut the rows to a height
})
```

A row is a list of runs, the shape `status.segments.draw` takes, and `sidebar.row(tokens, names,
it)` builds one out of a token table the way the tree does. Without a `fit` a section is cut by
`sidebar.scroll`, which keeps `anchor` on screen. A section whose `build` or `fit` raises is left
out of that frame and named once through `neotmux.notify`.
````

And replace the section from `### Changing what the sidebar looks like` up to (not including) `### Showing a pane's own history` (currently lines 1624–1642) with:

````markdown
### Changing what the sidebar looks like

`plugin.sidebar` takes the left of the screen and stacks its sections down it. In the tree, a ▸
marks the window being shown and a • the focused pane. Numbers are positions, the same way the
status bar writes them, so a pane reads `2.1`.

```lua
require('plugin.sidebar').colors.focused_pane = '#f9e2af'
require('plugin.sidebar.sessions').markers =
  { current_window = '>', window = ' ', focused_pane = '*', pane = ' ' }
```

`colors` and `markers` are read every tick. The colours are the sidebar's and every section reads
them. A marker is one cell: one that measured two columns would push every name on its row
across by one.

````

(The old closing paragraph about the width being a `local` is dropped: `widths` has been a field
since the band learned to resize.)

- [ ] **Step 8: Full check and commit**

Run: `make check`
Expected: fmt, clippy and the whole workspace suite pass.

```bash
git add runtime/lua/plugin/sidebar.lua runtime/lua/plugin/sidebar/sessions.lua \
  crates/neotmux-lua/tests/sidebar.rs docs/plugins.md
git commit -F - <<'EOF'
the sidebar is a stack of sections, and the session tree is the first of them

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 2: the tree gives up its panes before it gives up a session

**Files:**
- Modify: `runtime/lua/plugin/sidebar.lua` (`scroll`, new `marks`, `default_fit`)
- Modify: `runtime/lua/plugin/sidebar/sessions.lua` (new `fit`)
- Modify: `crates/neotmux-lua/tests/sidebar.rs`
- Modify: `docs/plugins.md` (the tree paragraph from Task 1)

**Interfaces:**
- Consumes: `build(sidebar)`'s `entries` and `tree` from Task 1.
- Produces: `sidebar.scroll(model, height, marks?)`, `sidebar.marks = { above = fn(n) -> runs, below = fn(n) -> runs }`, `sessions.fit(model, height, sidebar)`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/neotmux-lua/tests/sidebar.rs`:

```rust
/// `sidebar.scroll` on a section with no fit of its own: 20 rows centred on the tenth in 12,
/// rows 4..15, with the first and last of those given over to what was cut.
#[test]
fn the_default_fit_says_how_much_it_cut_on_each_side() {
    let (_dir, sub) = built(
        "local rows = {}\n\
         for i = 1, 20 do rows[i] = { { text = 'r' .. i } } end\n\
         require('plugin.sidebar').sections = {\n\
           { build = function() return { rows = rows, anchor = 10 } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert_eq!(lines[0], "\u{2191} 4", "{lines:#?}");
    assert_eq!(lines[1], "r5", "{lines:#?}");
    assert_eq!(lines[10], "r14", "{lines:#?}");
    assert_eq!(lines[11], "\u{2193} 6", "{lines:#?}");
    line_with(&lines, "r10");
}

/// Step 2: four sessions of two rows each leave four rows for a subtree of 21, centred on the
/// focused pane (row 6 of it): rows 4..7, the first and last of those given to the cut.
#[test]
fn every_session_stays_when_the_panes_do_not_fit() {
    let (_dir, sub) = built("");
    let s = [
        Sess::new(1, "alpha", vec![vec![101]]),
        Sess::new(2, "bravo", vec![(201..=220).collect()]).focused(205),
        Sess::new(3, "charlie", vec![vec![301]]),
        Sess::new(4, "delta", vec![vec![401]]),
    ];
    open(&sub, &s);
    let lines = settled(&sub, &s, 2);
    for name in ["alpha", "bravo", "charlie", "delta"] {
        line_with(&lines, name);
    }
    let above = line_with(&lines, "\u{2026} 4 more");
    let focused = line_with(&lines, "1.5 cat");
    let below = line_with(&lines, "\u{2026} 15 more");
    assert!(line_with(&lines, "bravo") < above, "{lines:#?}");
    assert!(above < focused && focused < below, "{lines:#?}");
    assert!(below < line_with(&lines, "charlie"), "{lines:#?}");
}

/// Step 3: ten sessions of two rows are more than twelve, so each is cut to its first row and
/// the subtree goes — every name is still there, and nothing says anything was cut.
#[test]
fn ten_sessions_with_a_crowd_of_panes_keep_every_name() {
    let (_dir, sub) = built("");
    let names = [
        "s00", "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09",
    ];
    let s: Vec<Sess> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let id = n as u32 + 1;
            let panes: Vec<u32> = if *name == "s03" {
                (1..=20).map(|p| id * 100 + p).collect()
            } else {
                vec![id * 100 + 1]
            };
            Sess::new(id, name, vec![panes])
        })
        .collect();
    open(&sub, &s);
    let lines = settled(&sub, &s, 4);
    for name in names {
        line_with(&lines, name);
    }
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("more") || l.contains('\u{2191}') || l.contains('\u{2193}')),
        "{lines:#?}"
    );
}

/// Step 4: twenty one-row sessions in twelve rows, centred on the eleventh.
#[test]
fn more_sessions_than_rows_scroll_around_the_one_you_are_in() {
    let (_dir, sub) = built("");
    let names = [
        "s00", "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09", "s10", "s11", "s12",
        "s13", "s14", "s15", "s16", "s17", "s18", "s19",
    ];
    let s: Vec<Sess> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let id = n as u32 + 1;
            Sess::new(id, name, vec![vec![id * 100 + 1]])
        })
        .collect();
    open(&sub, &s);
    let lines = settled(&sub, &s, 11);
    assert_eq!(lines[0], "\u{2191} 5", "{lines:#?}");
    assert_eq!(lines[11], "\u{2193} 5", "{lines:#?}");
    line_with(&lines, "s10");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p neotmux-lua --test sidebar`
Expected: the four new tests FAIL — no `↑`/`↓`/`… n more` rows are drawn, and in `every_session_stays_when_the_panes_do_not_fit` the centring pushes `alpha` off the top (`no line contains "alpha"`).

- [ ] **Step 3: Give `scroll` its marks**

In `runtime/lua/plugin/sidebar.lua`, replace `M.scroll` and `default_fit` with:

```lua
-- What `scroll` puts where it cut rows: `n` counts the row the mark itself stands in, so the
-- number is how many rows are not on screen on that side.
local function arrow(glyph)
  return function(n)
    return { { text = glyph .. " " .. n, fg = M.colors.detail, bg = M.colors.bar } }
  end
end

M.marks = { above = arrow("\u{2191}"), below = arrow("\u{2193}") }

-- The default fit: every row if they fit, and otherwise a window of them centred on `anchor`
-- and clamped to the ends, with the first and last rows of the window given over to saying how
-- much was cut on that side. A mark never stands in for the anchor, so the row that matters is
-- always one of the ones that stayed.
function M.scroll(model, height, marks)
  marks = marks or M.marks
  local rows = model.rows
  if height <= 0 then
    return {}
  end
  if #rows <= height then
    return rows
  end
  local anchor = model.anchor or 1
  local first = math.min(math.max(anchor - math.floor(height / 2), 1), #rows - height + 1)
  local last = first + height - 1
  local out = {}
  for at = first, last do
    out[#out + 1] = rows[at]
  end
  if first > 1 and first ~= anchor then
    out[1] = marks.above(first)
  end
  if last < #rows and last ~= anchor then
    out[#out] = marks.below(#rows - last + 1)
  end
  return out
end

-- `fit` is called with the sidebar as its third argument, which `scroll` would take for marks.
local function default_fit(model, height)
  return M.scroll(model, height)
end
```

- [ ] **Step 4: Give the tree its own fit**

In `runtime/lua/plugin/sidebar/sessions.lua`, add before `return M`:

```lua
-- What stands in for the subtree rows cut off either side of the pane you are in. Indented
-- like a tree row, so the first column stays the markers'.
local function more(n)
  return { { text = "  \u{2026} " .. n .. " more", fg = colors.detail, bg = colors.bar } }
end

-- Sessions before panes. Every row if they fit; else every session's rows with the subtree of
-- the one you are in cut to what is left around the focused pane; else each session at its first
-- row and no subtree; else those first rows scrolled around the one you are in. Where a session
-- is, is the thing the sidebar is for, and the panes of the one you are in are on your screen.
function M.fit(model, height, sidebar)
  colors = sidebar.colors
  if #model.rows <= height then
    return model.rows
  end
  local heads = 0
  for _, it in ipairs(model.entries) do
    heads = heads + #it.rows
  end
  if heads <= height then
    local cut = sidebar.scroll(model.tree, height - heads, { above = more, below = more })
    local out = {}
    for _, it in ipairs(model.entries) do
      for _, runs in ipairs(it.rows) do
        out[#out + 1] = runs
      end
      if it.here then
        for _, runs in ipairs(cut) do
          out[#out + 1] = runs
        end
      end
    end
    return out
  end
  local firsts, here = {}, 1
  for _, it in ipairs(model.entries) do
    if #it.rows > 0 then
      firsts[#firsts + 1] = it.rows[1]
      if it.here then
        here = #firsts
      end
    end
  end
  return sidebar.scroll({ rows = firsts, anchor = here }, height)
end
```

- [ ] **Step 5: Run the host-less and end-to-end tests**

Run: `cargo test -p neotmux-lua --test sidebar && cargo test -p neotmux --test fixtures sidebar`
Expected: all 11 host-less tests PASS; the 4 end-to-end sidebar tests PASS (`the_shipped_sidebar_keeps_the_pane_you_are_in_on_screen` now reaches step 2: the session's two rows plus seven of its ten subtree rows, window 5 among them).

- [ ] **Step 6: Say it in the docs**

In `docs/plugins.md`, after the paragraph ending `...says nothing about where you are.` (the tree paragraph from Task 1), add:

```markdown
When the tree does not fit, it gives up the windows and panes of the session you are in before
it gives up any session's name, and says `… n more` where it cut them. With more sessions than
rows it keeps one row each and scrolls them around the one you are in, with `↑ n` and `↓ n` for
the ones out of sight.
```

- [ ] **Step 7: Full check and commit**

Run: `make check`
Expected: pass.

```bash
git add runtime/lua/plugin/sidebar.lua runtime/lua/plugin/sidebar/sessions.lua \
  crates/neotmux-lua/tests/sidebar.rs docs/plugins.md
git commit -F - <<'EOF'
the tree gives up its panes before it gives up a session, and says what it cut

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 3: the detector says which ruleset answered

**Files:**
- Modify: `runtime/lua/neotmux/detect.lua` (`M.of`'s loop; new `M.agent`; header comment)
- Modify: `crates/neotmux-lua/tests/detect.rs`

**Interfaces:**
- Produces: `require('neotmux.detect').agent(pane) -> string | nil` — the name (key in `detect.agents`) of the ruleset whose rule matched at the last `of` look, nil where none did or `of` has not looked.

- [ ] **Step 1: Write the failing test**

Append to `crates/neotmux-lua/tests/detect.rs`:

```rust
/// `agent` names the ruleset that answered at the last look, and nothing once none does.
#[test]
fn agent_names_the_ruleset_that_answered_the_last_look() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         detect.agents = { fake = {\n\
           { id = 'busy', state = 'working', priority = 1,\n\
             match = function(v) return v.lines[#v.lines] == 'BUSY' end },\n\
         } }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local before = tostring(detect.agent(1))\n\
             detect.of(1)\n\
             neotmux.log.info('before=' .. before .. ' agent=' .. tostring(detect.agent(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"BUSY");
    keys(&sub, &["J"]);
    sub.terminals().feed(pane(), b"\x1b[2J\x1b[Hsomething else");
    keys(&sub, &["J"]);
    let said: Vec<String> = logged(&sub)
        .into_iter()
        .filter(|l| l.starts_with("before="))
        .collect();
    assert_eq!(
        said,
        vec!["before=nil agent=fake", "before=fake agent=nil"],
        "{said:?}"
    );
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p neotmux-lua --test detect agent_names`
Expected: FAIL — `attempt to call a nil value (field 'agent')`, so neither line is logged.

- [ ] **Step 3: Keep the name and hand it out**

In `runtime/lua/neotmux/detect.lua`, in `M.of`, change

```lua
          if id ~= nil then
            it.state, it.visible, it.matched = state, visible, true
            break
          end
```

to

```lua
          if id ~= nil then
            it.state, it.visible, it.matched, it.agent = state, visible, true, name
            break
          end
```

and add after `M.of` (before `return M`):

```lua
-- Which ruleset answered for the pane at the last look: its name in `M.agents`, or nil where
-- none did. Read off the cache `of` fills, so it costs nothing after `of` — and knows nothing of
-- a pane `of` has not been asked about.
function M.agent(pane)
  local it = seen[pane]
  return it and it.agent or nil
end
```

In the header comment, after the paragraph beginning `` `of` answers a second value too``, add:

```lua
-- `detect.agent(pane)` names the ruleset that answered at that look — `'claude'` — or nil, which
-- is how a list of agents tells an agent from a pane the fallback is guessing about.
```

- [ ] **Step 4: Run it to see it pass**

Run: `cargo test -p neotmux-lua --test detect && cargo test -p neotmux-lua --test state`
Expected: all PASS.

- [ ] **Step 5: Commit**

Run: `make check` — expected pass.

```bash
git add runtime/lua/neotmux/detect.lua crates/neotmux-lua/tests/detect.rs
git commit -F - <<'EOF'
the detector says which ruleset answered, so a list of agents can tell them from shells

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 4: the agents section

**Files:**
- Create: `runtime/lua/plugin/sidebar/agents.lua`
- Modify: `runtime/lua/plugin/sidebar.lua` (`M.sections`; header comment)
- Modify: `crates/neotmux-lua/tests/sidebar.rs`
- Modify: `docs/plugins.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `sidebar.row`, `sidebar.colors`, `sidebar.scroll` (via no `fit`), `detect.agent` (Task 3), `state.of`, `state.icon`.
- Produces (on `require('plugin.sidebar.agents')`): `title`, `share`, `rows`, `tokens` (`state_icon`, `location`, `command`, `age`), `clock`, `build(sidebar)`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/neotmux-lua/tests/sidebar.rs`:

```rust
/// A ruleset that knows three screens by their last line, and speaks only for panes running
/// `agent`; and a clock the test moves by three minutes on `T`.
const AGENTS: &str = "local detect = require('neotmux.detect')\n\
    detect.agents = { fake = {\n\
      claims = function(info) return info ~= nil and info.command == 'agent' end,\n\
      { id = 'ask', state = 'blocked', priority = 3, visible = 'blocked',\n\
        match = function(v) return v.lines[#v.lines] == 'ASK' end },\n\
      { id = 'busy', state = 'working', priority = 2,\n\
        match = function(v) return v.lines[#v.lines] == 'BUSY' end },\n\
      { id = 'done', state = 'idle', priority = 1, visible = 'idle',\n\
        match = function(v) return v.lines[#v.lines] == 'DONE' end },\n\
    } }\n\
    T = 0\n\
    require('plugin.sidebar.agents').clock = function() return T end\n\
    neotmux.on_key(function(key)\n\
      if key == 'T' then T = T + 180000 return true end\n\
      if key == 'B' then require('plugin.sidebar').toggle() return true end\n\
      return false\n\
    end)\n";

fn show(sub: &Built, pane: u32, text: &str) {
    sub.terminals().feed(
        PaneId::from(pane),
        format!("\x1b[2J\x1b[H{text}").as_bytes(),
    );
}

#[test]
fn agents_are_listed_across_sessions_in_state_order() {
    let (_dir, sub) = built(AGENTS);
    let s = [
        Sess::new(1, "work", vec![vec![101, 102]]).agents(&[101, 102]),
        Sess::new(2, "home", vec![vec![201, 202]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 102, "ASK");
    show(&sub, 201, "DONE");
    show(&sub, 202, "DONE");
    let lines = settled(&sub, &s, 1);
    let header = line_with(&lines, "\u{2500} agents \u{2500}");
    let blocked = line_with(&lines, "\u{25c9} work:1.2 agent 0s");
    let working = line_with(&lines, "\u{25cf} work:1.1 agent 0s");
    let idle = line_with(&lines, "\u{25cb} home:1.1 agent 0s");
    assert!(header < blocked && blocked < working && working < idle, "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("home:1.2")), "{lines:#?}");
}

#[test]
fn an_age_moves_on_with_the_clock_and_starts_over_on_a_change() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    line_with(&settled(&sub, &s, 1), "\u{25cf} work:1.1 agent 0s");
    support::keys(&sub, &["T"]);
    line_with(&settled(&sub, &s, 1), "\u{25cf} work:1.1 agent 3m");
    show(&sub, 101, "DONE");
    line_with(&settled(&sub, &s, 1), "\u{25cb} work:1.1 agent 0s");
    support::keys(&sub, &["T"]);
    line_with(&settled(&sub, &s, 1), "\u{25cb} work:1.1 agent 3m");
}

/// Review focus 5: an agent that quits leaves the list, and one that comes back is new.
#[test]
fn an_agent_that_quits_leaves_and_comes_back_new() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    line_with(&settled(&sub, &s, 1), "work:1.1 agent 0s");
    support::keys(&sub, &["T"]);
    show(&sub, 101, "$ ");
    let gone = settled(&sub, &s, 1);
    assert!(!gone.iter().any(|l| l.contains("agents")), "{gone:#?}");
    support::keys(&sub, &["T"]);
    show(&sub, 101, "BUSY");
    line_with(&settled(&sub, &s, 1), "work:1.1 agent 0s");
}

#[test]
fn no_agents_no_section() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let lines = settled(&sub, &s, 1);
    assert!(!lines.iter().any(|l| l.contains("agents")), "{lines:#?}");
}

/// Review focus 3: twelve agents in twelve rows. The list is held to half — a header and five,
/// the last of them `↓ 8` — and the tree keeps its session name in the other half.
#[test]
fn more_agents_than_half_the_band_stop_at_half() {
    let (_dir, sub) = built(AGENTS);
    let panes: Vec<u32> = (101..=112).collect();
    let s = [Sess::new(1, "work", vec![panes.clone()]).agents(&panes)];
    open(&sub, &s);
    for pane in &panes {
        show(&sub, *pane, "BUSY");
    }
    let lines = settled(&sub, &s, 1);
    assert!(lines.iter().any(|l| l.ends_with(" work")), "{lines:#?}");
    let header = line_with(&lines, "\u{2500} agents \u{2500}");
    assert_eq!(header, 6, "{lines:#?}");
    assert_eq!(lines[11], "\u{2193} 8", "{lines:#?}");
    let listed = lines.iter().filter(|l| l.contains("work:1.")).count();
    assert_eq!(listed, 4, "{lines:#?}");
}

/// Review focus 4: folded, the band is one column whatever the agent list has in it.
#[test]
fn a_folded_sidebar_is_one_column_with_agents_listed() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    settled(&sub, &s, 1);
    support::keys(&sub, &["B"]);
    let lines = settled(&sub, &s, 1);
    assert!(lines.iter().all(|l| l.chars().count() <= 1), "{lines:#?}");
    assert!(lines.iter().any(|l| l == "\u{25cf}"), "{lines:#?}");
}

/// Review focus 2: two rows. The agent list gets one, which is not room for its header and a
/// row, so it draws nothing; the tree gets the other and shows its session. Nothing is raised.
#[test]
fn a_terminal_too_short_for_a_header_draws_what_fits_and_raises_nothing() {
    let (_dir, sub) = built(AGENTS);
    let tiny = Size { rows: 2, cols: 80 };
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    frame_in(&sub, tiny, &s, 1);
    let lines = frame_in(&sub, tiny, &s, 1);
    assert!(lines[0].ends_with(" work"), "{lines:#?}");
    assert_eq!(lines[1], "", "{lines:#?}");
    let said: Vec<String> = support::notified(&sub)
        .into_iter()
        .filter(|t| t.contains("plugin.sidebar"))
        .collect();
    assert!(said.is_empty(), "{said:?}");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p neotmux-lua --test sidebar`
Expected: the new tests FAIL — `module 'plugin.sidebar.agents' not found` makes `run_init` return an error, and `support::built` panics.

- [ ] **Step 3: Write the section**

Create `runtime/lua/plugin/sidebar/agents.lua`:

```lua
-- The agents in every session, the second section of `plugin.sidebar`: each pane a ruleset in
-- `neotmux.detect.agents` answered for, what state it is in, and how long it has been in it —
-- the list herdr keeps down its side, so the one waiting for you is found without looking.
--
--   local agents = require('plugin.sidebar.agents')
--   agents.rows = { { 'state_icon', 'location', 'age' } }
--   agents.tokens.cwd = function(it) ... end
--
-- A pane no ruleset answered for is not here: a shell guessed at by `neotmux.state`'s pattern
-- list is in the tree above, and a list of agents with every shell in it is the tree again.
local detect = require("neotmux.detect")
local state = require("neotmux.state")

local M = { title = "agents", share = 0.5 }

M.rows = { { "state_icon", "location", "command", "age" } }

-- What `age` counts against, in milliseconds. A field so a test can hold the clock still.
M.clock = function()
  return neotmux.now()
end

-- The sidebar's colours, set by `build` for the tokens.
local colors

M.tokens = {
  state_icon = function(it)
    return state.icon(it.state), colors[it.state]
  end,
  location = function(it)
    return it.session .. ":" .. it.window .. "." .. it.pane,
      it.focused and colors.focused_pane or colors.window
  end,
  command = function(it)
    return it.command, colors.detail
  end,
  age = function(it)
    return it.age, colors.detail
  end,
}

local RANK = { blocked = 1, working = 2, idle = 3 }

-- Per pane, the state it is in and when that state was first seen. Rebuilt on every build out
-- of the panes that build found, so a pane that closed — in this session or another — or stopped
-- being an agent is dropped, and comes back new.
local since = {}

local function age(ms)
  local s = math.floor(ms / 1000)
  if s < 60 then
    return s .. "s"
  end
  if s < 3600 then
    return math.floor(s / 60) .. "m"
  end
  return math.floor(s / 3600) .. "h"
end

local function before(a, b)
  for k = 1, #a.order do
    if a.order[k] ~= b.order[k] then
      return a.order[k] < b.order[k]
    end
  end
  return false
end

function M.build(sidebar)
  colors = sidebar.colors
  local now = M.clock()
  local found, kept = {}, {}
  local here = neotmux.session.current()
  for session_at, name in ipairs(neotmux.session.list()) do
    neotmux.session.with(name, function()
      local focused = name == here and neotmux.pane.current() or nil
      for window_at, window in ipairs(neotmux.window.list()) do
        for pane_at, pane in ipairs(neotmux.window.panes(window) or {}) do
          -- `state.of` first: it is what asks the detector, and `agent` reads what it found.
          local now_state = state.of(pane)
          local agent = detect.agent(pane)
          if now_state and agent then
            local was = since[pane]
            if was == nil or was.state ~= now_state then
              was = { state = now_state, at = now }
            end
            kept[pane] = was
            local info = neotmux.pane.info(pane)
            found[#found + 1] = {
              state = now_state,
              session = name,
              window = window_at,
              pane = pane_at,
              command = info and info.command or agent,
              focused = pane == focused,
              age = age(now - was.at),
              order = { RANK[now_state] or 4, session_at, window_at, pane_at },
            }
          end
        end
      end
    end)
  end
  since = kept
  table.sort(found, before)

  local rows, anchor = {}, nil
  for _, it in ipairs(found) do
    if it.focused then
      anchor = #rows + 1
    end
    for at, names in ipairs(M.rows) do
      local runs = sidebar.row(M.tokens, names, it)
      if #runs > 0 then
        if at > 1 then
          table.insert(runs, 1, { text = "  ", bg = colors.bar })
        end
        rows[#rows + 1] = runs
      end
    end
  end
  return { rows = rows, anchor = anchor }
end

return M
```

- [ ] **Step 4: Ship it**

In `runtime/lua/plugin/sidebar.lua`, change

```lua
M.sections = {
  require("plugin.sidebar.sessions"),
}
```

to

```lua
M.sections = {
  require("plugin.sidebar.sessions"),
  require("plugin.sidebar.agents"),
}
```

and in the header comment change `The shipped one is the session tree, \`plugin.sidebar.sessions\`.` to:

```lua
-- cut them to a height. The shipped ones are the session tree, `plugin.sidebar.sessions`, and
-- under it every agent in every session, `plugin.sidebar.agents`.
```

(keep the sentence's first half, `A sidebar down the left: a stack of sections, each a table saying what rows it has and how to`, on the line above).

- [ ] **Step 5: Run the tests**

Run: `cargo test -p neotmux-lua --test sidebar && cargo test -p neotmux --test fixtures sidebar`
Expected: all 18 host-less tests PASS; the 4 end-to-end sidebar tests PASS (their panes run `cat`, and the shipped Claude rules match nothing on a `cat` screen, so no agents section appears). If an end-to-end test fails because an agents section appeared, stop and report it: it means a shipped Claude rule matches a bare `cat` pane, which is a detector finding, not something to paper over in this plan.

- [ ] **Step 6: Docs and changelog**

In `docs/plugins.md`, after the paragraph added in Task 2 (ending `...for the ones out of sight.`), add:

```markdown
Under the tree is every agent in every session — every pane a ruleset in
`neotmux.detect.agents` answered for — blocked first, then working, then idle, with how long each
has been in the state it is in:

    ◉ work:2.1 claude  3m

It is `require('plugin.sidebar.agents')`, with `rows` and `tokens` shaped like the tree's
(`state_icon`, `location`, `command`, `age`). With no agents it takes no room at all; with many
it takes at most half the band (`share = 0.5`) and the tree keeps the rest. An age starts when
the sidebar first sees the state, so one that was already going when neotmux started reads as at
least that long.
```

In `CHANGELOG.md`, under `## Unreleased`, add as the first entry:

```markdown
### The sidebar is a stack of sections, and the second one lists your agents

`plugin.sidebar` is a container now: `require('plugin.sidebar').sections` is a list of tables,
each with a `build` that says what rows it has and an optional `fit` that cuts them to a height,
and a user adds, drops or reorders them from `init.lua`. The session tree is the first; its
layout fields (`sessions`, `windows`, `panes`, `tokens`, `tree`, `markers`) moved with it to
`plugin.sidebar.sessions`, with no aliases left behind. The tree also stops losing sessions off
the top when the one you are in has many panes: it gives up those panes first, saying `… n more`
where it cut them. The second section, `plugin.sidebar.agents`, lists every pane an agent
ruleset answers for, in every session, blocked first, with how long it has been in its state —
which needed `neotmux.detect.agent(pane)`, the name of the ruleset that answered.
```

- [ ] **Step 7: Full check and commit**

Run: `make check`
Expected: pass.

```bash
git add runtime/lua/plugin/sidebar/agents.lua runtime/lua/plugin/sidebar.lua \
  crates/neotmux-lua/tests/sidebar.rs docs/plugins.md CHANGELOG.md
git commit -F - <<'EOF'
under the tree, every agent in every session and how long it has been waiting

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01W93pLmsQimhw3pUaRszQoZ
EOF
```

---

### Task 5: see it on a real screen

No code is kept from this task. Everything goes in the scratchpad.

**Files:**
- Create (scratch, not committed): `$SCRATCH/cfg/neotmux/init.lua`

- [ ] **Step 1: Build**

Run: `cargo build -p neotmux`
Expected: `target/debug/neotmux` built from the worktree.

- [ ] **Step 2: A config that makes a crowd**

Write `$SCRATCH/cfg/neotmux/init.lua` (where `$SCRATCH` is the session scratchpad):

```lua
require('neotmux.defaults').sidebar = true
local made = false
neotmux.on_key(function(key)
  if key == 'F5' and not made then
    made = true
    for i = 1, 9 do neotmux.session.create('s0' .. i) end
    for _ = 1, 14 do neotmux.pane.split('horizontal') end
    return true
  end
  return false
end)
```

- [ ] **Step 3: Ten sessions and a crowd of panes**

Run neotmux inside a detached tmux at 100×30 and capture:

```bash
rm -rf /tmp/ntsb && mkdir -p /tmp/ntsb
tmux -L ntsb new -d -x 100 -y 30 \
  "env XDG_CONFIG_HOME=$SCRATCH/cfg XDG_RUNTIME_DIR=/tmp/ntsb SHELL=/bin/bash $PWD/target/debug/neotmux"
sleep 2; tmux -L ntsb send-keys F5; sleep 2
tmux -L ntsb capture-pane -p > $SCRATCH/crowd.txt
```

Read `$SCRATCH/crowd.txt`. Expected: all ten session names down the left, the current one's subtree with `… n more` where it was cut, the focused pane's row visible. If `pane.split` refuses past some count, note how many panes there were rather than forcing more.

- [ ] **Step 4: A real agent**

```bash
tmux -L ntsb send-keys 'claude' Enter; sleep 8
tmux -L ntsb capture-pane -p > $SCRATCH/agent.txt
```

Read it. Expected: `─ agents ─` under the tree, with a row for the pane running Claude Code. Then ask it something that needs a permission prompt (for example `run: cat /etc/hostname`), wait, capture again, and check the row shows `◉` and an age that grows between two captures a minute apart.

- [ ] **Step 5: Clean up and report**

```bash
tmux -L ntsb kill-server; rm -rf /tmp/ntsb
```

Report what the captures showed — including anything that looked wrong — with the relevant lines of the capture quoted. Do not claim it works without those captures.

-- Every session, the upper section of `plugin.sidebar`: herdr's `spaces`, one entry a session —
-- what it is doing and what it is called, then the git branch it is on and how far that is from
-- its upstream.
--
--   local spaces = require('plugin.sidebar.spaces')
--   spaces.rows = { { 'state_icon', 'name' } }       -- one row, no git
--   spaces.refresh = 10000                           -- ask git every ten seconds, not five
--
-- `C-b w` (`navigate`) puts a cursor on it: `j`/`k` move it, Enter goes to the session under it,
-- `1`-`9` go straight to that one, Esc leaves. herdr's navigate mode, on the same key.
--
-- The tree this replaced, every window and pane of the session you are in, is still
-- `plugin.sidebar.sessions`, and goes back with one assignment to `require('plugin.sidebar')
-- .sections`.
local detect = require("neotmux.detect")
local sessions = require("plugin.sidebar.sessions")
local seen = require("neotmux.seen")
local state = require("neotmux.state")
local modes = require("neotmux.mode")

local M = { title = "spaces" }

M.rows = { { "state_icon", "name" }, { "branch", "ahead", "behind" } }
M.row_gap = 0

-- The windows and panes of the session you are in, under its space, drawn as
-- `plugin.sidebar.sessions` draws them — its `markers`, `tree` and `tokens` apply here too — to
-- these rows. Where a pane is, not what it runs: the agents below say that for the ones that
-- matter. `{ 'marker', 'tree', 'number', 'command' }` puts it back.
M.tree = true
M.tree_rows = {
  windows = { rows = { { "marker", "tree", "number", "name" } } },
  panes = { rows = { { "marker", "tree", "number" } } },
}

-- How often a directory's branch is asked again, in milliseconds.
M.refresh = 5000

-- How many directories one build may start asking about. A plugin may only have so many jobs
-- running at once, and a round that asked every directory together would fail the same ones
-- every time; a few at a time, the rest wait for the next build.
M.asks_per_build = 2
local budget = 0

-- The sidebar, set by `build` for the tokens.
local sidebar

M.tokens = {
  state_icon = function(it)
    return sidebar.icon(it.state), sidebar.colors[it.state]
  end,
  name = function(it)
    if it.here then
      return it.name, sidebar.colors.text, true
    end
    return it.name, sidebar.colors.subtext
  end,
  branch = function(it)
    return it.git and it.git.branch, it.here and sidebar.colors.branch or sidebar.colors.overlay
  end,
  ahead = function(it)
    local n = it.git and it.git.ahead or 0
    return n > 0 and ("\u{2191}" .. n) or nil, sidebar.colors.idle -- ↑, green
  end,
  behind = function(it)
    local n = it.git and it.git.behind or 0
    return n > 0 and ("\u{2193}" .. n) or nil, sidebar.colors.blocked -- ↓, red
  end,
}

-- herdr's rollup order: a session is what the most urgent of its agents is, and `unknown` when
-- it has none — a shell has no state worth a colour.
local RANK = { blocked = 1, done = 2, working = 3, idle = 4, unknown = 5 }

local function rollup(panes)
  local worst = "unknown"
  for _, pane in ipairs(panes) do
    -- `state.of` first: it is what asks the detector, and `agent` reads what it found.
    local one = seen.of(pane, state.of(pane))
    if one and detect.agent(pane) and RANK[one] and RANK[one] < RANK[worst] then
      worst = one
    end
  end
  return worst
end

-- Per directory: the branch and the counts, when they were asked, and whether an ask is out.
-- The sandbox has no `io`, so git is asked as a job, and a directory is asked at most every
-- `M.refresh` ms however many sessions sit in it.
local git = {}

local function ask(cwd)
  local known = git[cwd]
  local now = neotmux.now()
  if known and (known.busy or (known.at and now - known.at < M.refresh)) then
    return known
  end
  if budget <= 0 then
    return known
  end
  budget = budget - 1
  known = known or {}
  local asked_before = known.at
  known.busy, known.at = true, now
  git[cwd] = known
  local started = neotmux.job.start({ "git", "-C", cwd, "rev-parse", "--abbrev-ref", "HEAD" }, {
    on_exit = function(code, stdout)
      if code ~= 0 then
        known.branch, known.ahead, known.behind, known.busy = nil, 0, 0, false
        return
      end
      known.branch = (stdout or ""):match("^%s*(.-)%s*$")
      -- `@{upstream}...HEAD` counts what only the upstream has, then what only HEAD has: behind,
      -- then ahead. No upstream is a failure, and no counts.
      local counted = neotmux.job.start(
        { "git", "-C", cwd, "rev-list", "--left-right", "--count", "@{upstream}...HEAD" },
        {
          on_exit = function(status, out)
            local behind, ahead = (out or ""):match("(%d+)%s+(%d+)")
            if status == 0 and behind then
              known.behind, known.ahead = tonumber(behind), tonumber(ahead)
            else
              known.behind, known.ahead = 0, 0
            end
            known.busy = false
          end,
        }
      )
      if not counted then
        known.busy = false
      end
    end,
  })
  if not started then
    -- Not stamped: a start the job limit refused is asked again as soon as there is room,
    -- rather than a whole refresh later.
    known.busy, known.at = false, asked_before
  end
  return known
end

-- Where the navigate cursor is, by position in `session.list()`, or nil when it is not up.
M.cursor = nil

local function entries()
  budget = M.asks_per_build
  local out = {}
  local here = neotmux.session.current()
  for _, name in ipairs(neotmux.session.list()) do
    local it = { name = name, here = name == here }
    neotmux.session.with(name, function()
      -- Every window's panes, not `pane.list()`'s, which is the window on screen: an agent
      -- waiting in a window you are not showing is still that session waiting.
      local panes = {}
      for _, window in ipairs(neotmux.window.list()) do
        for _, pane in ipairs(neotmux.window.panes(window) or {}) do
          panes[#panes + 1] = pane
        end
      end
      it.state = rollup(panes)
      local focus = neotmux.pane.current()
      local info = focus and neotmux.pane.info(focus)
      if info and info.cwd then
        it.git = ask(info.cwd)
      end
    end)
    out[#out + 1] = it
  end
  return out
end

-- Shaped as `plugin.sidebar.sessions` shapes its model, each space's own rows and the tree hung
-- under the one you are in, so that section's fit cuts this one the same way.
function M.build(given)
  sidebar = given
  local rows, anchor, spaces = {}, 1, {}
  local tree = { rows = {} }
  for at, it in ipairs(entries()) do
    local selected = M.cursor == at
    local fill = selected and sidebar.colors.selection or it.here and sidebar.colors.here or nil
    if (it.here and not M.cursor) or selected then
      anchor = #rows + 1
    end
    local own = {}
    for line, names in ipairs(M.rows) do
      local runs = sidebar.row(M.tokens, names, it)
      if #runs > 0 then
        table.insert(runs, 1, { text = line == 1 and " " or "   " })
        runs.fill = fill
        own[#own + 1] = runs
        rows[#rows + 1] = runs
      end
    end
    spaces[#spaces + 1] = { rows = own, here = it.here }
    if it.here and M.tree then
      local base = #rows
      tree.anchor = sessions.subtree(sidebar, tree.rows, M.tree_rows)
      for _, runs in ipairs(tree.rows) do
        table.insert(runs, 1, { text = "   " })
        rows[#rows + 1] = runs
      end
      if tree.anchor and not M.cursor then
        anchor = base + tree.anchor
      end
    end
    for _ = 1, M.row_gap or 0 do
      rows[#rows + 1] = { { text = "", bg = sidebar.colors.bar } }
    end
  end
  return { rows = rows, anchor = anchor, entries = spaces, tree = tree }
end

-- With a tree, every space's name before any of its panes, as the tree section cuts; without
-- one, a window around the anchor.
function M.fit(model, height, given)
  if #model.tree.rows == 0 then
    return given.scroll(model, height)
  end
  return sessions.fit(model, height, given)
end

-- Folded: `1 ●`, a session a row, numbered as `1`-`9` in navigate go to them.
function M.rail(given)
  sidebar = given
  local rows = {}
  for at, it in ipairs(entries()) do
    rows[#rows + 1] = {
      { text = tostring(at), fg = it.here and sidebar.colors.text or sidebar.colors.overlay },
      { text = " " },
      { text = sidebar.icon(it.state), fg = sidebar.colors[it.state] },
      fill = it.here and sidebar.colors.here or nil,
    }
  end
  return rows
end

-- Navigate: the cursor, and the keys that move it and act on it, for as long as it is up.
local mode

local function leave()
  M.cursor = nil
  if mode then
    mode:lower()
    mode = nil
  end
  local bar = package.loaded["plugin.sidebar"]
  if bar and bar.navigating_unfolded then
    bar.open, bar.navigating_unfolded = false, nil
  end
end

local function go(at)
  local name = neotmux.session.list()[at]
  leave()
  if name and name ~= neotmux.session.current() then
    neotmux.session.switch(name)
  end
end

function M.navigate()
  local names = neotmux.session.list()
  local bar = package.loaded["plugin.sidebar"]
  local shown = false
  for _, section in ipairs(bar and bar.sections or {}) do
    shown = shown or section == M
  end
  -- A cursor over a section that is not drawn would swallow keys you cannot see it take.
  if #names == 0 or not shown then
    return
  end
  local here = neotmux.session.current()
  M.cursor = 1
  for at, name in ipairs(names) do
    if name == here then
      M.cursor = at
    end
  end
  if not bar.open then
    bar.open, bar.navigating_unfolded = true, true
  end
  if not mode then
    mode = modes.raise("navigate")
  end
end

local MOVES = { j = 1, Down = 1, k = -1, Up = -1 }

-- herdr's, along the bottom row while the cursor is up: that you are in it and what the keys do.
M.bar_colors = { mode = "#89b4fa", on = "#181825", text = "#6c7086" }

local function help_bar(ctx)
  local area = ctx:area()
  local y = area.y + area.h - 1
  local c = M.bar_colors
  ctx:rect(nil, area.x, y, area.w, 1, { bg = c.on })
  require("status.segments").draw(ctx, area.x, y, area.w, {
    { text = " NAVIGATE ", fg = c.on, bg = c.mode, bold = true },
    { text = " esc back  ", fg = c.text, bg = c.on },
    { text = "\u{2191}/\u{2193}", fg = c.mode, bg = c.on, bold = true },
    { text = " workspace", fg = c.text, bg = c.on },
  })
end

-- `C-b` while the cursor is up leaves it. The prefix sees the key first — it was loaded first —
-- and raises its mode, so this looks for that rather than for the key, which also follows a
-- prefix rebound to something else. What comes after the prefix key is then the prefix's.
neotmux.on_tick(function(ctx)
  if M.cursor and modes.current() == "prefix" then
    leave()
  end
  if M.cursor then
    help_bar(ctx)
  end
end)

neotmux.on_key(function(key)
  if not M.cursor then
    return false
  end
  local count = #neotmux.session.list()
  if count == 0 then
    leave()
    return true
  end
  -- A session that ended while the cursor was up can leave it past the end.
  M.cursor = math.min(M.cursor, count)
  if MOVES[key] then
    M.cursor = (M.cursor - 1 + MOVES[key]) % count + 1
  elseif key == "Enter" then
    go(M.cursor)
  elseif key:match("^[1-9]$") then
    go(tonumber(key))
  elseif key == "Esc" then
    leave()
  end
  -- Everything else is swallowed while the cursor is up, the way a list swallows it: a key
  -- meant for the cursor that went to the pane would be typed into a program.
  return true
end)

return M

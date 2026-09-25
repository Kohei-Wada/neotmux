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
local seen = require("neotmux.seen")
local state = require("neotmux.state")
local modes = require("neotmux.mode")

local M = { title = "spaces" }

M.rows = { { "state_icon", "name" }, { "branch", "ahead", "behind" } }
M.row_gap = 0

-- How often a directory's branch is asked again, in milliseconds.
M.refresh = 5000

-- The sidebar, set by `build` for the tokens.
local sidebar

M.tokens = {
  state_icon = function(it)
    return sidebar.icon(it.state), sidebar.colors[it.state]
  end,
  name = function(it)
    return it.name, it.here and sidebar.colors.current_session or sidebar.colors.session
  end,
  branch = function(it)
    return it.git and it.git.branch, sidebar.colors.current_window
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

-- herdr's rollup order: a session is what the most urgent of its panes is.
local RANK = { blocked = 1, done = 2, working = 3, idle = 4 }

local function rollup(panes)
  local worst = "idle"
  for _, pane in ipairs(panes) do
    local one = seen.of(pane, state.of(pane))
    if one and RANK[one] and RANK[one] < RANK[worst] then
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
  if known and (known.busy or now - known.at < M.refresh) then
    return known
  end
  known = known or {}
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
    known.busy = false
  end
  return known
end

-- Where the navigate cursor is, by position in `session.list()`, or nil when it is not up.
M.cursor = nil

local function entries()
  local out = {}
  local here = neotmux.session.current()
  for _, name in ipairs(neotmux.session.list()) do
    local it = { name = name, here = name == here }
    neotmux.session.with(name, function()
      it.state = rollup(neotmux.pane.list())
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

function M.build(given)
  sidebar = given
  local rows, anchor = {}, 1
  for at, it in ipairs(entries()) do
    local selected = M.cursor == at
    local bg = selected and sidebar.colors.selection or sidebar.colors.bar
    if it.here and not M.cursor then
      anchor = #rows + 1
    elseif selected then
      anchor = #rows + 1
    end
    for line, names in ipairs(M.rows) do
      local runs = sidebar.row(M.tokens, names, it)
      if #runs > 0 then
        table.insert(runs, 1, { text = line == 1 and " " or "   ", bg = bg })
        for _, run in ipairs(runs) do
          run.bg = bg
        end
        rows[#rows + 1] = runs
      end
    end
    for _ = 1, M.row_gap or 0 do
      rows[#rows + 1] = { { text = "", bg = sidebar.colors.bar } }
    end
  end
  return { rows = rows, anchor = anchor }
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
  if #names == 0 then
    return
  end
  local here = neotmux.session.current()
  M.cursor = 1
  for at, name in ipairs(names) do
    if name == here then
      M.cursor = at
    end
  end
  local bar = package.loaded["plugin.sidebar"]
  if bar and not bar.open then
    bar.open, bar.navigating_unfolded = true, true
  end
  if not mode then
    mode = modes.raise("navigate")
  end
end

local MOVES = { j = 1, Down = 1, k = -1, Up = -1 }

neotmux.on_key(function(key)
  if not M.cursor then
    return false
  end
  local count = #neotmux.session.list()
  if count == 0 then
    leave()
    return true
  end
  if MOVES[key] then
    M.cursor = (M.cursor - 1 + MOVES[key]) % count + 1
  elseif key == "Enter" then
    go(M.cursor)
  elseif key:match("^[1-9]$") then
    go(tonumber(key))
  elseif key == "Esc" or key == "C-b" then
    leave()
  end
  -- Everything else is swallowed while the cursor is up, the way a list swallows it: a key
  -- meant for the cursor that went to the pane would be typed into a program.
  return true
end)

return M

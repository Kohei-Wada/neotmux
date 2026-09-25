-- The agents in every session, the lower section of `plugin.sidebar`: each pane a ruleset in
-- `neotmux.detect.agents` answered for, and what state it is in — herdr's agent list, row for
-- row, so the one waiting for you is found without looking.
--
--   local agents = require('plugin.sidebar.agents')
--   agents.rows = { { 'state_icon', 'location', 'agent', 'age' } }
--   agents.sort = 'priority'
--   agents.tokens.cwd = function(it) ... end
--
-- A pane no ruleset answered for is not here: a shell is in the spaces above, and a list of
-- agents with every shell in it is the spaces again.
--
-- `collect()` is the list itself, in `sort` order, and is what the palette's agents and the
-- `agent-next` / `agent-prev` commands walk, so all three agree about which agent is next.
local detect = require("neotmux.detect")
local seen = require("neotmux.seen")
local state = require("neotmux.state")
local g = neotmux.group("plugin.sidebar.agents")

local M = { title = "agents", share = 0.5, always = true }

-- herdr's rows: where it is on the first, what it is on the second. `window` says nothing for a
-- session with one window, as herdr's `tab` does. `location`, `command`, `state_text` and `age`
-- are there to swap in.
M.rows = { { "state_icon", "session", "window" }, { "agent" } }

-- `'grouped'`: session, then window, then pane, herdr's default. `'priority'`: blocked, done,
-- working, idle, and session order within each.
M.sort = "grouped"

-- The header's right, as herdr's shows it.
M.aside = function()
  return M.sort
end

-- What `age` counts against, in milliseconds. A field so a test can hold the clock still.
M.clock = function()
  return neotmux.now()
end

-- The sidebar, set by `build` for the tokens.
local sidebar

M.tokens = {
  state_icon = function(it)
    return sidebar.icon(it.state), sidebar.colors[it.state]
  end,
  state_text = function(it)
    return it.state, sidebar.colors[it.state]
  end,
  session = function(it)
    return it.session, it.focused and sidebar.colors.text or sidebar.colors.subtext, true
  end,
  window = function(it)
    if it.windows < 2 then
      return nil
    end
    return it.window_name or tostring(it.window), sidebar.colors.subtext
  end,
  location = function(it)
    return it.session .. ":" .. it.window .. "." .. it.pane,
      it.focused and sidebar.colors.focused_pane or sidebar.colors.window
  end,
  agent = function(it)
    return it.agent, sidebar.colors.overlay
  end,
  command = function(it)
    return it.command, sidebar.colors.overlay
  end,
  age = function(it)
    return it.age, sidebar.colors.overlay
  end,
}

local RANK = { blocked = 1, done = 2, working = 3, idle = 4 }

-- Per pane, the state it is in and when that state was first seen. Rebuilt on every collect out
-- of the panes that collect found, so a pane that closed — in this session or another — or
-- stopped being an agent is dropped, and comes back new.
local since = {}

-- Three columns whatever the count, because the band is as wide as its widest row: an age that
-- grew a digit would widen it, and every pane would be resized each time one rolled over.
local function age(ms)
  local s = math.floor(ms / 1000)
  local text
  if s < 60 then
    text = s .. "s"
  elseif s < 3600 then
    text = math.floor(s / 60) .. "m"
  else
    text = math.floor(s / 3600) .. "h"
  end
  return string.format("%3s", text)
end

local function before(a, b)
  for k = 1, #a.order do
    if a.order[k] ~= b.order[k] then
      return a.order[k] < b.order[k]
    end
  end
  return false
end

-- Every agent there is, in `M.sort` order. Each is what a row is built from, plus the ids a
-- caller needs to go there: `pane_id`, `window_id` and `session`.
function M.collect()
  local now = M.clock()
  local found, kept = {}, {}
  local here = neotmux.session.current()
  for session_at, name in ipairs(neotmux.session.list()) do
    neotmux.session.with(name, function()
      local focused = name == here and neotmux.pane.current() or nil
      local windows = neotmux.window.list()
      for window_at, window in ipairs(windows) do
        for pane_at, pane in ipairs(neotmux.window.panes(window) or {}) do
          -- `state.of` first: it is what asks the detector, and `agent` reads what it found.
          local now_state = seen.of(pane, state.of(pane))
          local agent = detect.agent(pane)
          if now_state and agent then
            local was = since[pane]
            if was == nil or was.state ~= now_state then
              was = { state = now_state, at = now }
            end
            kept[pane] = was
            local info = neotmux.pane.info(pane)
            local rank = M.sort == "priority" and (RANK[now_state] or 5) or 0
            found[#found + 1] = {
              state = now_state,
              session = name,
              here = name == here,
              window = window_at,
              window_name = neotmux.window.name(window),
              windows = #windows,
              pane = pane_at,
              pane_id = pane,
              window_id = window,
              agent = agent,
              command = info and info.command or agent,
              focused = pane == focused,
              age = age(now - was.at),
              order = { rank, session_at, window_at, pane_at },
            }
          end
        end
      end
    end)
  end
  since = kept
  table.sort(found, before)
  return found
end

-- Goes to an agent `collect()` handed out: shows the window it is in and focuses it, in its own
-- session — an op raised inside `session.with` is that session's — and then moves there.
function M.focus(it)
  neotmux.session.with(it.session, function()
    neotmux.window.focus(it.window_id)
    neotmux.pane.focus(it.pane_id)
  end)
  if it.session ~= neotmux.session.current() then
    neotmux.session.switch(it.session)
  end
end

-- The agent after (or before) the one you are in, in `M.sort` order, wrapping; the first when
-- you are in none. herdr's `next_agent` / `previous_agent`, and like them bound to nothing.
local function step(by)
  local all = M.collect()
  if #all == 0 then
    return
  end
  local at
  for index, it in ipairs(all) do
    if it.focused then
      at = index
    end
  end
  local next_at = at and ((at - 1 + by) % #all + 1) or 1
  M.focus(all[next_at])
end

g.command.add("agent-next", function()
  step(1)
end, { desc = "go to the next agent" })
g.command.add("agent-prev", function()
  step(-1)
end, { desc = "go to the previous agent" })

function M.build(given)
  sidebar = given
  local rows, anchor = {}, nil
  for _, it in ipairs(M.collect()) do
    if it.focused then
      anchor = #rows + 1
    end
    for at, names in ipairs(M.rows) do
      local runs = sidebar.row(M.tokens, names, it)
      if #runs > 0 then
        table.insert(runs, 1, { text = at == 1 and " " or "   " })
        runs.fill = it.focused and sidebar.colors.here or nil
        rows[#rows + 1] = runs
      end
    end
  end
  return { rows = rows, anchor = anchor }
end

-- Folded: `1 ●`, an agent a row.
function M.rail(given)
  sidebar = given
  local rows = {}
  for at, it in ipairs(M.collect()) do
    rows[#rows + 1] = {
      { text = tostring(at), fg = it.focused and sidebar.colors.text or sidebar.colors.overlay },
      { text = " " },
      { text = sidebar.icon(it.state), fg = sidebar.colors[it.state] },
      fill = it.focused and sidebar.colors.here or nil,
    }
  end
  return rows
end

return M

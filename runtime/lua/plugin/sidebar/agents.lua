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

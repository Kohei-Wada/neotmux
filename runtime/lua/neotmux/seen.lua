-- Whether you have seen an agent finish: the `done` state herdr shows, an agent that went idle
-- while nobody was looking at it, until somebody does.
--
--   local seen = require('neotmux.seen')
--   seen.of(pane, state.of(pane))   --> the state, or 'done'
--
-- Called inside `neotmux.session.with` for the pane's own session, the way the sidebar reads
-- every pane. A pane is looked at when it is in the current window of a session a client is
-- attached to (`neotmux.session.attached`): every session ticks whether or not anybody is there,
-- so a session nobody is at does not count as being read.
--
-- Only a pane an agent ruleset answers for (`neotmux.detect.agent`) is ever done. A shell goes
-- idle every time a command finishes, and a sidebar lit up by every one of them says nothing.
--
-- Not a detector's answer, which is why it is not in `neotmux.state`: the chime and the watcher
-- ask what a pane is doing, and this is about what you have and have not looked at.
local detect = require("neotmux.detect")

local M = {}

-- The state each pane was last read in, and the panes that finished unwatched.
local last = {}
local unseen = {}

local function looked_at(pane)
  local name = neotmux.session.current()
  if name == nil or neotmux.session.attached(name) == 0 then
    return false
  end
  local window = neotmux.window.current()
  for _, shown in ipairs(window and neotmux.window.panes(window) or {}) do
    if shown == pane then
      return true
    end
  end
  return false
end

function M.of(pane, state)
  if state == nil then
    return nil
  end
  local was = last[pane]
  last[pane] = state
  if state ~= "idle" or looked_at(pane) then
    unseen[pane] = nil
  elseif (was == "working" or was == "blocked") and detect.agent(pane) then
    unseen[pane] = true
  end
  return unseen[pane] and "done" or state
end

-- What a caller that has just walked every pane there is keeps: everything else is a pane that
-- has closed, and is dropped.
function M.keep(alive)
  for pane in pairs(last) do
    if not alive[pane] then
      last[pane] = nil
      unseen[pane] = nil
    end
  end
end

return M

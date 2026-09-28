-- Whether you have seen an agent finish: the `done` state herdr shows, an agent that went idle
-- while nobody was looking at it, until somebody does.
--
--   local seen = require('neotmux.seen')
--   seen.of(pane, state.of(pane))   --> the state, or 'done'
--
-- `of` only reads; what a pane went through is recorded by this module's own tick, which runs in
-- every session every frame. A pane is looked at when it is in the current window of a session a
-- client is attached to (`neotmux.session.attached`): every session ticks whether or not anybody
-- is there, so a session nobody is at does not count as being read.
--
-- Only a pane an agent ruleset answers for (`neotmux.detect.agent`) is ever done. A shell goes
-- idle every time a command finishes, and a sidebar lit up by every one of them says nothing.
--
-- Not a detector's answer, which is why it is not in `neotmux.state`: the chime and the watcher
-- ask what a pane is doing, and this is about what you have and have not looked at.
local detect = require("neotmux.detect")
local state = require("neotmux.state")
local g = neotmux.group("neotmux.seen")

local M = {}

-- The state each pane was last seen in, the panes that finished unwatched, and which session
-- each pane is in, so a session's tick can drop the ones it no longer has.
local last = {}
local unseen = {}
local owner = {}

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

-- Its own tick, one per session per frame, over that session's panes: what a pane went through
-- is recorded here whether or not anything is drawing it, so the palette on `C-b g` knows with
-- the sidebar off, and a reader that last looked long ago is not mistaken for a transition.
g.on_tick(function()
  local name = neotmux.session.current()
  if name == nil then
    return
  end
  local present = {}
  for _, pane in ipairs(require("neotmux.panes").all()) do
    present[pane] = true
    owner[pane] = name
    local now = state.of(pane)
    local was = last[pane]
    last[pane] = now
    if now ~= "idle" or looked_at(pane) then
      unseen[pane] = nil
    elseif (was == "working" or was == "blocked") and detect.agent(pane) then
      unseen[pane] = true
    end
  end
  for pane, who in pairs(owner) do
    if who == name and not present[pane] then
      owner[pane], last[pane], unseen[pane] = nil, nil, nil
    end
  end
end)

-- `state`, or `'done'` for an idle pane that finished while nobody was looking. A read: it
-- changes nothing, however often and from wherever it is asked.
function M.of(pane, now)
  if now == "idle" and unseen[pane] then
    return "done"
  end
  return now
end

return M

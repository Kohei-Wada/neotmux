-- Says which pane looks like it is waiting for you.
--
-- Off by default, unlike the other shipped plugins: a status line is a fact about the
-- terminal, while what counts as "waiting for a human" is an opinion about what you run. Turn
-- it on from your own `init.lua`:
--
--   require('neotmux.defaults').watch = true
--
-- The guess itself is not here: it is `neotmux.state`, so that the plugin which notifies you
-- and the sidebar which draws you a list agree about what a pane is doing. Narrow the patterns
-- there:
--
--   require('neotmux.state').patterns = { 'my agent is waiting' }
--
-- Nothing here is a host feature. It is `neotmux.window.panes` read from a tick, the shared
-- guess, and `neotmux.notify`.
local state = require("neotmux.state")
local g = neotmux.group("plugin.herdr.watch")

local M = {}

-- The pane you are looking at is the one you can already see is waiting.
M.ignore_focused = true

-- Which panes were waiting last time they were read, so the notification is sent on the change
-- rather than on every frame the prompt is still up.
local waiting = {}

g.on_event("pane_closed", function(event)
  waiting[event.pane] = nil
end)

g.on_tick(function()
  local focused = M.ignore_focused and neotmux.pane.current() or nil
  -- Every pane the session owns, hidden windows included: an agent you started in a window you
  -- are not looking at has said nothing yet, and that is exactly the pane worth watching.
  for _, window in ipairs(neotmux.window.list()) do
    for _, pane in ipairs(neotmux.window.panes(window) or {}) do
      local now = state.of(pane) == "blocked"
      if now and not waiting[pane] and pane ~= focused then
        neotmux.notify("info", "pane " .. pane .. " looks like it is waiting for you")
      end
      waiting[pane] = now
    end
  end
end)

return M

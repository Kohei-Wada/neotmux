-- Every pane the session holds.
--
-- A convention, not a plugin: it registers nothing.
local M = {}

-- Every window's panes in window order, each window's in layout order, hidden windows included:
-- `neotmux.pane.list()` is only the window on the screen, and the agent waiting in a window you
-- are not looking at is the one worth knowing about.
function M.all()
  local out = {}
  for _, window in ipairs(neotmux.window.list()) do
    for _, pane in ipairs(neotmux.window.panes(window) or {}) do
      out[#out + 1] = pane
    end
  end
  return out
end

return M

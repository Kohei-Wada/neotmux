-- A real two-key prefix, bound to nothing: `on_key` is what lets it see the command key
-- it never registered, and the modal flag is the plugin's own state, not the host's.
--
-- The key and the command table are both fields of the table this file returns, so a user
-- can rebind, add or drop a command without editing this file:
--
--   local prefix = require('plugin.prefix')
--   prefix.key = 'C-a'
--   prefix.commands['%'] = prefix.commands['v']   -- tmux's muscle memory, alongside v and -
--   prefix.commands['"'] = prefix.commands['-']
--   prefix.commands['o'] = nil                    -- and one dropped
--
-- Both are read inside the callback rather than captured at load time, which is what makes
-- such an assignment take effect on the next key instead of never.
local armed = false

-- "The next pane" out of what the ABI offers: `list()` is in layout order and `current()`
-- says where we are, so the successor is an index step with a wrap.
local function focus_next()
  local ids = neotmux.pane.list()
  local current = neotmux.pane.current()
  for index, id in ipairs(ids) do
    if id == current then
      neotmux.pane.focus(ids[index % #ids + 1])
      return
    end
  end
end

-- The same index step over `window.list()`, which is in creation order.
local function next_window()
  local ids = neotmux.window.list()
  local current = neotmux.window.current()
  for index, id in ipairs(ids) do
    if id == current then
      neotmux.window.focus(ids[index % #ids + 1])
      return
    end
  end
end

-- `split('vertical')` divides the width, so `v` puts the two panes side by side; `-` divides
-- the height, which is what the glyph looks like.
local M = {
  key = "C-b",
  commands = {
    ["v"] = function()
      neotmux.pane.split("vertical")
    end,
    ["-"] = function()
      neotmux.pane.split("horizontal")
    end,
    ["o"] = focus_next,
    -- `create()` answers with the id before the window exists, so the two ops go out in one
    -- tick and are applied in that order: opened, then shown.
    ["c"] = function()
      neotmux.window.focus(neotmux.window.create())
    end,
    ["n"] = next_window,
  },
}

neotmux.on_key(function(key)
  if armed then
    armed = false
    local command = M.commands[key]
    if command then
      command()
    end
    return true
  end
  if key == M.key then
    armed = true
    return true
  end
  return false
end)

return M

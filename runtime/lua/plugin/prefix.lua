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
--
--   v  -        split the focused pane, side by side or stacked
--   o           the next pane in layout order
--   h j k l     the pane to the left, below, above, to the right
--   r           resize mode: arrows or h/j/k/l move the boundary, anything else leaves
--   c  n  p     a new window, the next one, the one before
local nav = require("neotmux.nav")
local modes = require("neotmux.mode")

-- Two modes, not one: the armed prefix and resize mode are different states, and a reader
-- should be able to tell them apart. Each local holds the handle `neotmux.mode.raise` hands
-- back while its mode is up, and nil while it is not — the handle doubles as the flag the
-- flag used to be, so there is no state kept twice.
local armed = nil
local resizing = nil

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

-- The same index step over `window.list()`, which is in creation order. One step either way,
-- wrapping at both ends, so `n` and `p` are each other's undo with one window or with ten.
local function step_window(step)
  return function()
    local ids = neotmux.window.list()
    local current = neotmux.window.current()
    for index, id in ipairs(ids) do
      if id == current then
        neotmux.window.focus(ids[(index - 1 + step) % #ids + 1])
        return
      end
    end
  end
end

local next_window = step_window(1)
local prev_window = step_window(-1)

-- A direction key, in both spellings a hand reaches for. `h/j/k/l` move the focus; the same
-- four, and the arrows, move a boundary once `r` has been pressed.
local DIRECTIONS = {
  h = "left",
  j = "down",
  k = "up",
  l = "right",
  Left = "left",
  Down = "down",
  Up = "up",
  Right = "right",
}

-- The pane that way, or nothing at all: at the edge of the window `h` does nothing rather
-- than wrapping round to the far side. A wrap is a jump across the whole window, and the
-- reason to press `h` is that you are looking at what is to your left; `o` is still there
-- for walking every pane in order.
local function focus_toward(dir)
  return function()
    local id = nav.toward(dir)
    if id then
      neotmux.pane.focus(id)
    end
  end
end

-- `split('vertical')` divides the width, so `v` puts the two panes side by side; `-` divides
-- the height, which is what the glyph looks like.
local M = {
  key = "C-b",
  -- How many cells one press of an arrow moves the boundary.
  resize_step = 1,
  commands = {
    ["v"] = function()
      neotmux.pane.split("vertical")
    end,
    ["-"] = function()
      neotmux.pane.split("horizontal")
    end,
    ["o"] = focus_next,
    ["h"] = focus_toward("left"),
    ["j"] = focus_toward("down"),
    ["k"] = focus_toward("up"),
    ["l"] = focus_toward("right"),
    ["r"] = function()
      resizing = modes.raise("resize")
    end,
    -- `create()` answers with the id before the window exists, so the two ops go out in one
    -- tick and are applied in that order: opened, then shown.
    ["c"] = function()
      neotmux.window.focus(neotmux.window.create())
    end,
    ["n"] = next_window,
    ["p"] = prev_window,
  },
}

neotmux.on_key(function(key)
  if resizing then
    local dir = DIRECTIONS[key]
    if dir then
      neotmux.pane.resize(dir, M.resize_step)
    else
      -- One rule, so nothing can strand you in the mode: a key that is not an adjustment
      -- leaves it and does nothing else. Esc and Enter are simply the two that mean nothing
      -- else anywhere.
      resizing:lower()
      resizing = nil
    end
    return true
  end
  if armed then
    armed:lower()
    armed = nil
    local command = M.commands[key]
    if command then
      command()
    end
    return true
  end
  if key == M.key then
    armed = modes.raise("prefix")
    return true
  end
  return false
end)

return M

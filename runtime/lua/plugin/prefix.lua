-- A real two-key prefix, bound to nothing: `on_key` is what lets it see the command key
-- it never registered, and the modal flag is the plugin's own state, not the host's.
--
-- The key and the command table are both fields of the table this file returns, so a user
-- can rebind, add or drop a command without editing this file. Every verb here is also a
-- named `neotmux.command`, so `commands` holds the name rather than the body, and the palette
-- lists every one of these on a fresh install:
--
--   local prefix = require('plugin.prefix')
--   prefix.key = 'C-a'
--   prefix.commands['%'] = prefix.commands['v']   -- tmux's muscle memory, alongside v and -
--   prefix.commands['"'] = prefix.commands['-']
--   prefix.commands['o'] = nil                    -- and one dropped
--   prefix.commands['!'] = function()             -- and one of your own, unnamed
--     neotmux.notify('info', 'panes: ' .. #neotmux.pane.list())
--   end
--
-- Both are read inside the callback rather than captured at load time, which is what makes
-- such an assignment take effect on the next key instead of never.
--
--   v  -        split the focused pane, side by side or stacked
--   o           the next pane in layout order
--   h j k l     the pane to the left, below, above, to the right
--   z           zoom the pane over the whole window, and back
--   r           resize mode: h/j/k/l move the boundary, anything else leaves
--   c  n  p     a new window, the next one, the one before
--   ,           name the window you are in; an empty name leaves it its number
--   d           leave the session running and let go of this terminal
local nav = require("neotmux.nav")
local modes = require("neotmux.mode")
local prompt = require("neotmux.prompt")
local g = neotmux.group("plugin.prefix")

-- Two modes, not one: the armed prefix and resize mode are different states, and a reader
-- should be able to tell them apart. Each local holds the handle `neotmux.mode.raise` hands
-- back while its mode is up, and nil while it is not — the handle doubles as the flag the
-- flag used to be, so there is no state kept twice.
local armed = nil
local resizing = nil

-- "The next pane" out of what the ABI offers: `window.panes` is in layout order and `current()`
-- says where we are, so the successor is an index step with a wrap. The window's panes rather
-- than `pane.list()`, which is only the zoomed pane while one is: focusing a pane the zoom
-- hides unzooms, and `o` walks every pane either way.
local function focus_next()
  local ids = neotmux.window.panes(neotmux.window.current()) or neotmux.pane.list()
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

-- A direction key. `h/j/k/l` move the focus, and the same four move a boundary once `r` has
-- been pressed. The arrows are not here on purpose: one spelling, in every mode neotmux
-- ships, is worth more than two in this one.
local DIRECTIONS = {
  h = "left",
  j = "down",
  k = "up",
  l = "right",
}

-- The pane that way, or nothing at all: at the edge of the window `h` does nothing rather
-- than wrapping round to the far side. A wrap is a jump across the whole window, and the
-- reason to press `h` is that you are looking at what is to your left; `o` is still there
-- for walking every pane in order.
--
-- While a pane is zoomed nothing is that way on screen, so the first press unzooms and the
-- next one moves.
local function focus_toward(dir)
  return function()
    if neotmux.pane.zoomed() then
      neotmux.pane.zoom()
      return
    end
    local id = nav.toward(dir)
    if id then
      neotmux.pane.focus(id)
    end
  end
end

-- `split('vertical')` divides the width, so `v` puts the two panes side by side; `-` divides
-- the height, which is what the glyph looks like.
local function split_vertical()
  neotmux.pane.split("vertical")
end

local function split_horizontal()
  neotmux.pane.split("horizontal")
end

local function zoom()
  neotmux.pane.zoom()
end

local function begin_resize()
  resizing = modes.raise("resize")
end

-- `create()` answers with the id before the window exists, so the two ops go out in one
-- tick and are applied in that order: opened, then shown.
local function new_window()
  neotmux.window.focus(neotmux.window.create())
end

local function rename_window()
  local id = neotmux.window.current()
  if id == nil then
    return
  end
  prompt.open({
    title = "name window",
    value = neotmux.window.name(id) or "",
    mode = "rename",
    -- An empty name is not a name: it takes the one there was away and leaves the window
    -- the number it had before anybody called it anything.
    done = function(text)
      neotmux.window.rename(id, text ~= "" and text or nil)
    end,
  })
end

local function leave_session()
  neotmux.detach()
end

-- Every verb, registered once under a name a palette or a CLI can call it by. `M.commands`
-- below still maps the key that runs each one, but now to the name rather than the body: one
-- definition, two ways in, rather than the body copied into a second table.
--
-- A refusal — another layer's `init.lua`, which runs first, already holding the name — is
-- first-wins on purpose and stays that way; what is not on purpose is losing the news of it.
-- `why` is kept here, by name, so it can be said once `M.commands` below is built and the key
-- that silently lost its binding is known.
local refused = {}
local function register(name, fn, desc)
  local ok, why = g.command.add(name, fn, { desc = desc })
  if not ok then
    refused[name] = why
  end
end

register("split-vertical", split_vertical, "split the focused pane, side by side")
register("split-horizontal", split_horizontal, "split the focused pane, one above the other")
register("focus-next", focus_next, "focus the next pane in layout order")
register("focus-left", focus_toward("left"), "focus the pane to the left")
register("focus-down", focus_toward("down"), "focus the pane below")
register("focus-up", focus_toward("up"), "focus the pane above")
register("focus-right", focus_toward("right"), "focus the pane to the right")
register("zoom", zoom, "zoom the focused pane over the whole window, or unzoom")
register(
  "resize",
  begin_resize,
  "enter resize mode: h/j/k/l move the boundary, anything else leaves"
)
register("window-new", new_window, "open a new window and switch to it")
register("window-next", next_window, "switch to the next window")
register("window-prev", prev_window, "switch to the window before this one")
register("window-rename", rename_window, "name the window you are in")
register("detach", leave_session, "leave the session running and let go of this terminal")

local M = {
  key = "C-b",
  -- How many cells one press moves the boundary.
  resize_step = 1,
  -- The key-to-name mapping: a key runs the command its entry names. An entry may also hold a
  -- function directly, for a plugin like `plugin.sessions` that binds a key of its own without
  -- registering one — `dispatch`, below, takes either.
  commands = {
    ["v"] = "split-vertical",
    ["-"] = "split-horizontal",
    ["o"] = "focus-next",
    ["h"] = "focus-left",
    ["j"] = "focus-down",
    ["k"] = "focus-up",
    ["l"] = "focus-right",
    ["r"] = "resize",
    ["z"] = "zoom",
    ["c"] = "window-new",
    ["n"] = "window-next",
    ["p"] = "window-prev",
    [","] = "window-rename",
    ["d"] = "detach",
  },
}

-- Said now, with the key named alongside the name: silently, `C-b d` would run whichever
-- layer got `detach` first, and the person pressing it has no way to know it was not this one.
for key, name in pairs(M.commands) do
  if refused[name] then
    neotmux.notify("warn", M.key .. " " .. key .. ": " .. refused[name])
  end
end

-- A key's entry is a command name, run through `neotmux.command.run` so it is one definition
-- shared with the palette, or a function, for an entry a plugin assigned directly rather than
-- through the registry.
local function dispatch(key)
  local target = M.commands[key]
  if type(target) == "function" then
    target()
  elseif type(target) == "string" then
    local ok, why = neotmux.command.run(target)
    if not ok then
      neotmux.notify("error", why)
    end
  end
end

g.on_key(function(key)
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
    dispatch(key)
    return true
  end
  if key == M.key then
    armed = modes.raise("prefix")
    return true
  end
  return false
end)

return M

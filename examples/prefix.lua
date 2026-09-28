-- A tmux-style two-key prefix that binds no key at all.
--
-- `keymap.set` can only see keys it has registered, which is not enough for a prefix: the
-- key *after* the prefix is whatever the user pressed. `on_key` sees every key before any
-- binding runs, so the prefix can swallow the pair and leave every other key alone.
--
-- The modal state is an ordinary Lua upvalue. neotmux has no idea what a prefix is.

local g = neotmux.group("plugin.tmux.prefix")

local M = { key = "C-a" }

local armed = false

local function focus_next()
  local ids = neotmux.pane.list() -- layout order
  local current = neotmux.pane.current()
  for index, id in ipairs(ids) do
    if id == current then
      neotmux.pane.focus(ids[index % #ids + 1])
      return
    end
  end
end

local commands = {
  ["%"] = function()
    neotmux.pane.split("vertical")
  end,
  ['"'] = function()
    neotmux.pane.split("horizontal")
  end,
  ["o"] = focus_next,
  ["c"] = function()
    neotmux.notify("info", "no pane can be closed from Lua yet")
  end,
}

g.on_key(function(key)
  if armed then
    armed = false
    local command = commands[key]
    if command then
      command()
    end
    -- Swallow the second key whether or not it was a command, so a mistyped prefix does
    -- not leak a stray character into the shell.
    return true
  end
  if key == M.key then
    armed = true
    return true
  end
  return false
end)

return M

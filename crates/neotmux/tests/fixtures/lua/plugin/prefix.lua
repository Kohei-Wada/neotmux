-- A real two-key prefix, bound to nothing: `on_key` is what lets it see the command key
-- it never registered, and the modal flag is the plugin's own state, not the host's.
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

local commands = {
  ["%"] = function()
    neotmux.pane.split("vertical")
  end,
  ['"'] = function()
    neotmux.pane.split("horizontal")
  end,
  ["o"] = focus_next,
}

neotmux.on_key(function(key)
  if armed then
    armed = false
    neotmux.log.info("prefix command " .. key)
    local command = commands[key]
    if command then
      command()
    end
    return true
  end
  if key == "C-a" then
    armed = true
    neotmux.log.info("prefix armed")
    return true
  end
  return false
end)

return {}

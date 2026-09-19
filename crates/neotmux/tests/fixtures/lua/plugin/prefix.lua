-- A real two-key prefix, bound to nothing: `on_key` is what lets it see the command key
-- it never registered, and the modal flag is the plugin's own state, not the host's.
local armed = false

neotmux.on_key(function(key)
  if armed then
    armed = false
    neotmux.log.info("prefix command " .. key)
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

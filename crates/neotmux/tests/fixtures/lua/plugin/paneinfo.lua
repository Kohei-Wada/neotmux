-- What a pane is doing, the way a status line reads it.
--
-- Logged on change rather than every tick: a tick runs sixty times a second, while what it
-- reports changes when a command starts or a directory does.
local last = {}

local function line(id)
  local info = neotmux.pane.info(id)
  if not info then
    return "paneinfo " .. id .. " nil"
  end
  return "paneinfo " .. id .. " command=" .. tostring(info.command) .. " cwd=" .. tostring(info.cwd)
end

neotmux.on_tick(function()
  for _, id in ipairs(neotmux.pane.list()) do
    local now = line(id)
    if last[id] ~= now then
      last[id] = now
      neotmux.log.info(now)
    end
  end
  if not last.unknown then
    last.unknown = true
    neotmux.log.info("paneinfo unknown=" .. tostring(neotmux.pane.info(4242)))
  end
end)

return {}

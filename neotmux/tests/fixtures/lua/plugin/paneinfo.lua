-- What a pane is doing, the way a status line reads it.
--
-- Every pane the session holds rather than `pane.list()`: `pane.info` describes a pane in a
-- window nobody is looking at too, and that is the half a list of the shown window cannot
-- reach.
--
-- Logged on change rather than every tick: a tick runs sixty times a second, while what it
-- reports changes when a command starts or a directory does.
local last = {}
local seen = {}

local function all_panes()
  local ids = {}
  for _, window in ipairs(neotmux.window.list()) do
    for _, id in ipairs(neotmux.window.panes(window) or {}) do
      ids[#ids + 1] = id
    end
  end
  return ids
end

local function line(id)
  local info = neotmux.pane.info(id)
  if not info then
    return "paneinfo " .. id .. " nil"
  end
  return "paneinfo " .. id .. " command=" .. tostring(info.command) .. " cwd=" .. tostring(info.cwd)
end

neotmux.on_tick(function()
  local live = {}
  for _, id in ipairs(all_panes()) do
    live[id] = true
    seen[id] = true
    local now = line(id)
    if last[id] ~= now then
      last[id] = now
      neotmux.log.info(now)
    end
  end
  -- A pane that has gone is asked about once more: a stale entry left behind would answer
  -- with whatever it was running when it died.
  for id in pairs(seen) do
    if not live[id] then
      seen[id] = nil
      last[id] = nil
      neotmux.log.info("paneinfo gone " .. id .. "=" .. tostring(neotmux.pane.info(id)))
    end
  end
  if not last.unknown then
    last.unknown = true
    neotmux.log.info("paneinfo unknown=" .. tostring(neotmux.pane.info(4242)))
  end
end)

return {}

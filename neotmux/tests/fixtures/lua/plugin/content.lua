-- What a pane is showing, read as lines from both places a watcher would read it: the tick,
-- and the `pane_output` handler the chunk arrives in.
--
-- The panes it knows about come from `pane_output` rather than `pane.list()`, because a pane
-- in a hidden window is in no list and is exactly the one worth watching.
local seen = {}
local asked_unknown = false

local function nonblank(lines)
  local out = {}
  for _, line in ipairs(lines) do
    if line ~= "" then
      out[#out + 1] = line
    end
  end
  return out
end

local function report(pane, tag)
  local lines, revision = neotmux.pane.content(pane)
  if lines == nil then
    neotmux.log.info("content " .. pane .. " " .. tag .. " nil")
    return
  end
  neotmux.log.info(
    "content "
      .. pane
      .. " "
      .. tag
      .. " rev="
      .. revision
      .. " ["
      .. table.concat(nonblank(lines), "|")
      .. "]"
  )
end

neotmux.on_event("pane_output", function(event)
  if seen[event.pane] == nil then
    seen[event.pane] = false
  end
  -- The chunk has already reached the pane's terminal by the time the event is dispatched,
  -- so what this reads includes it.
  report(event.pane, "output")
end)

neotmux.on_event("pane_closed", function(event)
  seen[event.pane] = nil
  -- The pane is gone by the time this is dispatched, so its terminal must be gone with it.
  neotmux.log.info(
    "content " .. event.pane .. " closed=" .. tostring(neotmux.pane.content(event.pane))
  )
end)

neotmux.on_tick(function()
  for pane in pairs(seen) do
    local revision = neotmux.pane.revision(pane)
    if revision == nil then
      neotmux.log.info("content " .. pane .. " gone")
      seen[pane] = nil
    elseif revision ~= seen[pane] then
      seen[pane] = revision
      report(pane, "tick")
    end
  end
  if not asked_unknown then
    asked_unknown = true
    neotmux.log.info("content unknown=" .. tostring(neotmux.pane.content(4242)))
  end
end)

return {}

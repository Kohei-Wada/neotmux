-- Window actions as a plugin sees them: keys this plugin swallows, turned into
-- `neotmux.window` calls. Nothing here is a host keybinding.
local function join(ids)
  local parts = {}
  for index, id in ipairs(ids) do
    parts[index] = tostring(id)
  end
  return table.concat(parts, ",")
end

-- What the session looks like from inside a handler, which is the whole point of reporting
-- it: an event must describe the session as it is, not as it was at the top of the frame.
local function seen()
  return " windows="
    .. join(neotmux.window.list())
    .. " current="
    .. tostring(neotmux.window.current())
    .. " panes="
    .. join(neotmux.pane.list())
end

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

neotmux.on_key(function(key)
  if key == "C-w" then
    -- The id comes back before the window exists, so both ops go out in one callback.
    neotmux.window.focus(neotmux.window.create())
    return true
  end
  if key == "C-t" then
    neotmux.log.info("windows created " .. neotmux.window.create())
    return true
  end
  if key == "C-n" then
    next_window()
    return true
  end
  if key == "C-y" then
    neotmux.window.focus(99)
    return true
  end
  if key == "C-x" then
    neotmux.window.kill(neotmux.window.current())
    return true
  end
  if key == "C-r" then
    neotmux.log.info("windows report" .. seen())
    return true
  end
  return false
end)

-- Every chunk, whichever window's pane produced it: a hidden pane's output being consumed
-- while it is hidden is the difference between a window that goes on working and one that is
-- merely caught up with on the way back.
neotmux.on_event("pane_output", function(event)
  neotmux.log.info("windows output " .. event.pane)
end)

neotmux.on_event("window_opened", function(event)
  neotmux.log.info("windows window_opened " .. event.window .. seen())
end)

neotmux.on_event("window_closed", function(event)
  neotmux.log.info("windows window_closed " .. event.window .. seen())
end)

neotmux.on_event("window_changed", function(event)
  neotmux.log.info("windows window_changed " .. event.window .. seen())
end)

return {}

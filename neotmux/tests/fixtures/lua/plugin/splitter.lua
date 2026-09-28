-- Pane actions as a plugin sees them: keys this plugin swallows, turned into `neotmux.pane`
-- calls. Nothing here is a host keybinding; the host never hears of C-b, C-o or C-g.
neotmux.on_key(function(key)
  if key == "C-b" then
    neotmux.pane.split("vertical")
    return true
  end
  if key == "C-o" then
    local current = neotmux.pane.current()
    for _, id in ipairs(neotmux.pane.list()) do
      if id ~= current then
        neotmux.pane.focus(id)
        break
      end
    end
    return true
  end
  if key == "C-k" then
    neotmux.pane.kill(neotmux.pane.current())
    return true
  end
  if key == "C-u" then
    neotmux.pane.focus(99)
    return true
  end
  if key == "C-g" then
    neotmux.log.info(
      "splitter panes=" .. #neotmux.pane.list() .. " current=" .. tostring(neotmux.pane.current())
    )
    return true
  end
  return false
end)

-- What the tree looks like from inside the handler, which is the whole point of reporting it:
-- an event must describe the session as it is, not as it was at the top of the frame.
local function seen()
  return " panes=" .. #neotmux.pane.list() .. " current=" .. tostring(neotmux.pane.current())
end

neotmux.on_event("pane_opened", function(event)
  neotmux.log.info("splitter pane_opened " .. event.pane .. seen())
end)

neotmux.on_event("pane_closed", function(event)
  neotmux.log.info("splitter pane_closed " .. event.pane .. seen())
end)

neotmux.on_event("focus_changed", function(event)
  neotmux.log.info("splitter focus_changed " .. event.pane)
end)

return {}

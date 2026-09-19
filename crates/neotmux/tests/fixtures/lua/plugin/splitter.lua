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

neotmux.on_event("pane_opened", function(event)
  neotmux.log.info("splitter pane_opened " .. event.pane)
end)

neotmux.on_event("pane_closed", function(event)
  neotmux.log.info("splitter pane_closed " .. event.pane)
end)

neotmux.on_event("focus_changed", function(event)
  neotmux.log.info("splitter focus_changed " .. event.pane)
end)

return {}

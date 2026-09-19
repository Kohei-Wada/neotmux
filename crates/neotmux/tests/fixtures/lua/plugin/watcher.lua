-- pane_output is the one event carrying a payload, so this is where a plugin finds out
-- what the shell actually wrote.
neotmux.on_event("pane_output", function(event)
  if event.bytes:find("WATCH%-ME", 1) then
    neotmux.log.info("watcher saw WATCH-ME on pane " .. event.pane)
  end
end)

return {}

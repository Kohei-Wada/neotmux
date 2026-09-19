-- Every `resized` event, logged. A resize is the host telling every pane its grid changed,
-- so counting them from a plugin's seat is how a test sees a layout recompute nobody asked
-- for: only a real terminal resize and the status row appearing or disappearing fire one.
neotmux.on_event("resized", function(event)
  neotmux.log.info("resized " .. event.rows .. "x" .. event.cols)
end)

return {}

-- A status provider that reports the surface it was given: without ctx:size() a status
-- plugin cannot know its own width (§4.5).
neotmux.on_tick(function(ctx)
  local size = ctx:size()
  ctx:status("CLOCK " .. size.cols .. "x" .. size.rows)
end)

return {}

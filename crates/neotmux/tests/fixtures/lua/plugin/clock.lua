-- A status provider that reports the surface it was given: without ctx:size() a status
-- plugin cannot know its own width (§4.5).
neotmux.on_tick(function(ctx)
  local cols, rows = ctx:size()
  ctx:status("CLOCK " .. cols .. "x" .. rows)
end)

return {}

-- A band plugin that reports the surface the panes were left with: without ctx:size() a
-- plugin drawing along an edge cannot know its own width (§4.5).
neotmux.reserve("bottom", 1)

neotmux.on_tick(function(ctx)
  local band = ctx:reserved()
  local size = ctx:size()
  if band then
    ctx:text(nil, band.x, band.y, "CLOCK " .. size.cols .. "x" .. size.rows)
  end
end)

return {}

-- The client's side of the float, announced each tick so a test can watch a reattach.
local win = nil
neotmux.on_tick(function(ctx)
  if not win then
    win = neotmux.float({ x = 4, y = 3, w = 12, h = 4 })
  end
  if win then
    ctx:text(win, 0, 0, "float window")
    local r = ctx:float_rect(win)
    if r then
      neotmux.log.info("float " .. r.x .. "," .. r.y .. " " .. r.w .. "x" .. r.h)
    end
  end
end)
return {}

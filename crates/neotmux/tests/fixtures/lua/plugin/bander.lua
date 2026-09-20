-- A band asked for and given back while the session runs, so a test can watch the panes
-- lose a row and take it back on screen. C-r reserves, C-t releases.
local M = { handle = nil }

neotmux.keymap.set("C-r", function()
  if not M.handle then
    M.handle = neotmux.reserve("bottom", 1)
  end
  return true
end)

neotmux.keymap.set("C-t", function()
  if M.handle then
    M.handle:remove()
    M.handle = nil
  end
  return true
end)

neotmux.on_tick(function(ctx)
  local band = ctx:reserved()
  if band then
    ctx:text(nil, band.x, band.y, "BAND")
  end
end)

return M

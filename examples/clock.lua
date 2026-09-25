-- A clock along the bottom of the screen.
--
-- It asks the host for one row and draws in it. The host has no status-line concept: it
-- reserves the band, takes it off what the panes get, and never learns what goes there.
--
-- The format is read from a table this file returns, so a user can change it without
-- editing the plugin:
--
--   require('plugin.clock').format = '%H:%M'
--
-- Reading it inside the tick rather than capturing it at load time is what makes that
-- assignment take effect on the next frame instead of never.

local g = neotmux.group("plugin.clock")

local M = { format = "%H:%M:%S" }

M.band = g.reserve("bottom", 1)

g.on_tick(function(ctx)
  local band = ctx:reserved()
  -- Nil on a terminal too small to spare the row: the panes keep it and the clock waits.
  if band then
    ctx:text(nil, band.x, band.y, os.date(M.format), { fg = 4 })
  end
end)

return M

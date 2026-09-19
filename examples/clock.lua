-- A clock in the status line.
--
-- The format is read from a table this file returns, so a user can change it without
-- editing the plugin:
--
--   require('plugin.clock').format = '%H:%M'
--
-- Reading it inside the tick rather than capturing it at load time is what makes that
-- assignment take effect on the next frame instead of never.

local M = { format = '%H:%M:%S' }

neotmux.on_tick(function(ctx)
  ctx:status(os.date(M.format), { fg = 4 })
end)

return M

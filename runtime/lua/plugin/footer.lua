-- A second reserved band, along the bottom, showing whichever mode is raised through
-- `neotmux.mode` — nothing more.
--
-- It reads the convention and never learns who raised what it shows: a third-party plugin
-- that calls `neotmux.mode.raise` on its own name shows up here with no change to this file,
-- the same way a third-party status module would show up if it drew into the top band.
--
-- Off by default, for the same kind of reason `watch` and `sidebar` are: it spends a row,
-- whether or not a mode is ever raised into it.
local modes = require("neotmux.mode")
local g = neotmux.group("plugin.footer")

local M = {}

-- Catppuccin Mocha, matching `plugin.status`. Read every tick, so assigning to one from your
-- own `init.lua` takes effect.
M.colors = {
  bar = "#181825", -- mantle: the same band colour the status bar sits on
  text = "#f9e2af", -- yellow: a mode is a thing to notice, not to blend in
}

M.band = g.reserve("bottom", 1)

g.on_tick(function(ctx)
  local row = ctx:reserved()
  if not row then
    return
  end
  -- The band is painted whether or not there is a mode to name. A row nobody reserved belongs
  -- to the panes; a row this plugin reserved and left alone is a strip of bare terminal under
  -- everything else, which reads as a hole rather than as an empty bar.
  ctx:rect(nil, row.x, row.y, row.w, row.h, { bg = M.colors.bar })
  local name = modes.current()
  if not name then
    return
  end
  ctx:text(nil, row.x, row.y, " " .. name .. " ", { fg = M.colors.text, bg = M.colors.bar })
end)

return M

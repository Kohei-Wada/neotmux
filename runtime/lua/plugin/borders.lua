-- The dividers between panes.
--
-- The host has no idea what a border is. What it offers is `pane_rect`, `line`, and panes
-- that tile the surface exactly — and "exactly" is the whole trick: a pane edge that is not
-- an edge of the surface is shared with a neighbour, so which panes touch never has to be
-- asked for. A pane whose right edge is not the surface's has somebody on the other side of
-- it, and that is all a divider needs to know. Every divider is drawn once, by the pane on
-- its left or above.
--
-- Tiling also means there is no gap to put a divider in, so it goes on the boundary — the
-- trailing column or row of the pane that owns it. That is a cell the shell there could have
-- used: a border costs a column, because the host hands panes the whole surface.
local M = { fg = 8 }

neotmux.on_tick(function(ctx)
  local size = ctx:size()
  local colours = { fg = M.fg }
  for _, id in ipairs(neotmux.pane.list()) do
    local r = ctx:pane_rect(id)
    -- A pane the terminal has shrunk to nothing has no edge to draw on.
    if r and r.w > 0 and r.h > 0 then
      if r.x + r.w < size.cols then
        ctx:line(nil, r.x + r.w - 1, r.y, r.x + r.w - 1, r.y + r.h - 1, colours)
      end
      if r.y + r.h < size.rows then
        ctx:line(nil, r.x, r.y + r.h - 1, r.x + r.w - 1, r.y + r.h - 1, colours)
      end
    end
  end
end)

return M

-- The dividers between panes.
--
-- The host reserves the space; this plugin draws in it. `neotmux.opt.pane_gutter` says
-- "leave this many cells between panes", the same deal as a status fragment saying "leave a
-- row": the host takes the cells away from the panes and never learns what goes in them. So
-- a divider costs no pane a column of its shell's output.
--
-- Which panes touch still never has to be asked for. With a gutter of g, a pane's right
-- neighbour starts at `r.x + r.w + g`, so the g columns from `r.x + r.w` are the gap between
-- the two — and a pane whose right edge is not the surface's has somebody on the other side
-- of that gap. Every divider is drawn once, by the pane on its left or above.
local GUTTER = 1

-- Every field is read on the tick, so assigning to one from your own init.lua takes effect.
-- The glyphs are as thin as a terminal draws: a cell cannot be subdivided, so one cell of
-- box drawing is the whole budget.
local M = { fg = 8, vertical = "│", horizontal = "─" }

neotmux.opt.pane_gutter = GUTTER

neotmux.on_tick(function(ctx)
  local size = ctx:size()
  local colours = { fg = M.fg }
  for _, id in ipairs(neotmux.pane.list()) do
    local r = ctx:pane_rect(id)
    -- A pane the terminal has shrunk to nothing has no neighbour worth a divider.
    if r and r.w > 0 and r.h > 0 then
      if r.x + r.w < size.cols then
        ctx:line(nil, r.x + r.w, r.y, r.x + r.w, r.y + r.h - 1, colours, M.vertical)
      end
      if r.y + r.h < size.rows then
        ctx:line(nil, r.x, r.y + r.h, r.x + r.w - 1, r.y + r.h, colours, M.horizontal)
      end
    end
  end
end)

return M

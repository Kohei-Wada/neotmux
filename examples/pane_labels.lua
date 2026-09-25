-- Writes each pane's id in its top-left corner, and marks the focused one.
--
-- Drawing is immediate-mode: what this tick draws is the whole of this plugin's output for
-- the frame. There is no clear call, and none is needed — stop drawing a label and it is
-- gone on the next frame.
--
-- `ctx:text(pane, ...)` places text in that pane's coordinate space, so the label follows
-- the pane through splits and resizes without this plugin tracking geometry itself.

local g = neotmux.group("plugin.pane_labels")

local M = { focused_fg = 2, other_fg = 8 }

g.on_tick(function(ctx)
  local current = neotmux.pane.current()
  for _, id in ipairs(neotmux.pane.list()) do
    local rect = ctx:pane_rect(id)
    -- A pane too small to hold its label is skipped rather than drawn over.
    if rect and rect.w >= 4 then
      local fg = (id == current) and M.focused_fg or M.other_fg
      ctx:text(id, 0, 0, "[" .. id .. "]", { fg = fg })
    end
  end
end)

return M

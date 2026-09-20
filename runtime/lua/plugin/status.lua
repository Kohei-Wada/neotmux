-- The default status line.
--
-- It lives here, and not in the host, because the host does not know what a status line is.
-- What it knows is how to take a band of rows away from the panes on request: `reserve`
-- returns a handle like every other registration, the panes get what is left, and nothing a
-- shell writes can land in the band while nothing in the band can be overwritten by a pane.
--
-- Drawing into it is the ordinary `ctx:text` with no pane, because `ctx:reserved()` answers
-- in the surface's coordinates — the same ones a `nil` pane draws in.
local M = {}

M.band = neotmux.reserve('bottom', 1)

neotmux.on_tick(function(ctx)
  local row = ctx:reserved()
  -- Nil while the terminal is too small to spare the row. The panes have it back, so there
  -- is nowhere to draw and nothing to draw over.
  if not row then
    return
  end
  local panes = neotmux.pane.list()
  ctx:text(nil, row.x, row.y, 'panes ' .. #panes .. ' focus ' .. tostring(neotmux.pane.current()))
end)

return M

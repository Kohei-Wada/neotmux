-- What a plugin that draws in a band down every pane's edge needs, besides what it draws there:
-- the band itself, reserved and given back on a toggle, the rule down its last column, and the
-- panes to keep track of while the band is hidden.
--
-- A convention, not a plugin: it registers nothing. `plugin.stamps` and `plugin.exits` are
-- built on it.
--
--   local gutter = require('neotmux.gutter')
--   local g = neotmux.group('plugin.mine')
--   local M = { shown = false, width = 6 }
--   M.toggle = gutter.toggler(M, 'plugin.mine', function(width)
--     return g.reserve_pane('left', width)
--   end)
--   g.on_tick(function(ctx)
--     for _, pane in ipairs(neotmux.pane.list()) do
--       local rect = ctx:pane_band(pane)
--       if rect then
--         gutter.rule(ctx, rect, '#45475a')
--       end
--     end
--   end)
local M = {}

-- The most columns every pane band together may take, as the host refuses past it.
M.max_width = 24

-- Every window's panes, the hidden ones too: the pane in a window you are not looking at is the
-- one whose record you will want when you come back to it. The shown panes as well, for a host
-- that has published panes but no windows.
function M.every_pane()
  local out = require("neotmux.panes").all()
  local listed = {}
  for _, pane in ipairs(out) do
    listed[pane] = true
  end
  for _, pane in ipairs(neotmux.pane.list()) do
    if not listed[pane] then
      out[#out + 1] = pane
    end
  end
  return out
end

-- A function that shows `state`'s band or hides it: `reserve(width)` on the way in, and the band
-- given back on the way out, with `state.shown` true only while the band is held. A width out of
-- range, or a band the host will not fit beside the others, leaves it hidden and says why, under
-- `name`.
--
-- `reserve` is the plugin's own function calling its own group's `reserve_pane`, never this
-- file's: a band belongs to whichever file asks for it, and `ctx:pane_band` answers it to that
-- one alone.
function M.toggler(state, name, reserve)
  local band
  return function()
    state.shown = not state.shown
    if not state.shown then
      if band then
        band:remove()
        band = nil
      end
      return
    end
    if state.width < 1 or state.width > M.max_width then
      state.shown = false
      neotmux.notify(
        "warn",
        name .. ": width must be 1.." .. M.max_width .. ", got " .. tostring(state.width)
      )
      return
    end
    local handle, message = reserve(state.width)
    if handle == nil then
      state.shown = false
      neotmux.notify("warn", name .. ": " .. message)
      return
    end
    band = handle
  end
end

-- The rule down a band's last column, which is what sets it apart from the pane beside it.
function M.rule(ctx, rect, colour)
  local x = rect.x + rect.w - 1
  for row = 0, rect.h - 1 do
    ctx:text(nil, x, rect.y + row, "│", { fg = colour })
  end
end

return M

-- A sidebar down the left: every window, every window's panes, and what each pane is
-- running. `C-b b` folds it to the markers column and back, when the prefix is loaded.
--
-- Off by default, unlike the status bar: a status line spends a row nobody was using for
-- anything else, while a sidebar is an opinion about screen real estate — twenty-four columns
-- is a third of an eighty-column terminal. Turn it on from your own `init.lua`:
--
--   require('neotmux.defaults').sidebar = true
--
-- Nothing here is a host feature. `neotmux.reserve('left', n)` takes the band,
-- `handle:resize_to(n)` folds it, `ctx:reserved()` answers where it landed, and the rest is
-- `window.list`, `window.panes`, `pane.info` and `pane.current` read from a tick.
local segments = require("status.segments")

local M = {}

-- The two states the band is ever in, both read when it is swapped, so assigning to either
-- from your own `init.lua` takes effect on the next fold.
--
-- Closed is a rail rather than nothing: one column is the marker column, so folded you still
-- see how many windows and panes there are and which of them you are in — you just cannot
-- read their names. A band of nothing would be a band you have to remember is there.
M.widths = {
  open = 24,
  closed = 1,
}

-- Catppuccin Mocha, the palette the status bar was drawn from. Every field is read on the
-- tick, so assigning to one from your own init.lua takes effect.
M.colors = {
  bar = "#181825", -- mantle: the band everything else sits on
  window = "#cdd6f4",
  current_window = "#cba6f7", -- mauve, the status bar's accent for the window being shown
  pane = "#6c7086", -- overlay0: a pane in a window you are not looking at
  focused_pane = "#a6e3a1", -- green
}

-- The column of markers down the left, so a row says what it is before it says which one.
-- One cell each, and never a combining or double-width glyph: a marker that measures two
-- columns would push every name on that row out by one.
M.markers = {
  current_window = "\u{25b8}", -- ▸
  window = " ",
  focused_pane = "\u{2022}", -- •
  pane = " ",
}

M.band = neotmux.reserve("left", M.widths.open)

-- Which of the two the band is in. The rows are the same either way: `segments.draw` cuts a
-- run that does not fit, so the rail is the open sidebar with everything but the markers
-- clipped off, and there is no second way of drawing this to keep in step with the first.
M.open = true

-- Swap the two. A resize keeps the band's place on the left edge, which a remove and a fresh
-- reservation would not: folding would walk the sidebar inside anybody who reserved after it.
function M.toggle()
  -- Nil if the left edge was already spoken for when this file loaded.
  if not M.band then
    return
  end
  local want = M.open and M.widths.closed or M.widths.open
  if M.band:resize_to(want) then
    M.open = not M.open
  end
end

-- The prefix is the standard plugin and this one is optional, so this is the direction the
-- knowledge goes in: bound only if the prefix is already loaded, which it is by the time the
-- sidebar loads unless it was turned off.
if package.loaded["plugin.prefix"] then
  require("plugin.prefix").commands["b"] = M.toggle
end

-- What a pane is running, or nil where the host has not got it.
--
-- `pane.info` answers about the panes of the window being shown and no others — reading
-- /proc for panes nobody can see is a syscall bill that grows with the windows — so a pane
-- in a hidden window has no command here. That is the host's decision, not a gap this plugin
-- works around.
local function command(pane)
  local info = neotmux.pane.info(pane)
  return info and info.command
end

-- What a pane row says when the host has no command for the pane.
local UNKNOWN = "\u{2014}" -- em dash

-- One row of the sidebar: a marker, an indent, a number, and for a pane its command.
local function row(marker, indent, number, name, colour)
  local text = marker .. string.rep(" ", indent) .. number
  if name then
    text = text .. " " .. name
  end
  return { { text = text, fg = colour, bg = M.colors.bar } }
end

-- The whole sidebar as a list of rows, top to bottom. Built before anything is drawn so the
-- band can simply be filled until it runs out of rows.
local function rows()
  local current_window = neotmux.window.current()
  local focused = neotmux.pane.current()
  local out = {}
  for window_at, window in ipairs(neotmux.window.list()) do
    local is_current = window == current_window
    out[#out + 1] = row(
      is_current and M.markers.current_window or M.markers.window,
      0,
      tostring(window_at),
      nil,
      is_current and M.colors.current_window or M.colors.window
    )
    -- Nil for a window that closed between the list and now, which is not an error worth a
    -- sidebar with a hole in it.
    for pane_at, pane in ipairs(neotmux.window.panes(window) or {}) do
      local is_focused = pane == focused
      out[#out + 1] = row(
        is_focused and M.markers.focused_pane or M.markers.pane,
        1,
        window_at .. "." .. pane_at,
        command(pane) or UNKNOWN,
        is_focused and M.colors.focused_pane or M.colors.pane
      )
    end
  end
  return out
end

neotmux.on_tick(function(ctx)
  local band = ctx:reserved()
  -- Nil while the terminal is too narrow to spare the columns. The panes have them back, so
  -- there is nowhere to draw and nothing to draw over.
  if not band then
    return
  end
  ctx:rect(nil, band.x, band.y, band.w, band.h, { bg = M.colors.bar })

  -- More windows than rows is a session, not a mistake: the rows that do not fit are dropped
  -- rather than drawn past the bottom of the band into a pane.
  for at, runs in ipairs(rows()) do
    if at > band.h then
      break
    end
    segments.draw(ctx, band.x, band.y + at - 1, band.w, runs)
  end
end)

return M

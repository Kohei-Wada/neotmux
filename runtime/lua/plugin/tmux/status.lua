-- The default status bar, on one reserved row across the top: herdr's tab bar, with a
-- neotmux window where herdr has a tab.
--
-- It lives here, and not in the host, because the host does not know what a status bar is.
-- What it knows is how to take a band of rows away from the panes on request: `reserve`
-- returns a handle like every other registration, the panes get what is left, and nothing a
-- shell writes can land in the band while nothing in the band can be overwritten by a pane.
--
-- Drawing into it is the ordinary `ctx:text` with no pane, because `ctx:reserved()` answers
-- in the surface's coordinates — the same ones a `nil` pane draws in.
--
-- A port of herdr's src/client/shell/tabs.rs, less the mouse's buttons; NOTICE says which.
local segments = require("status.segments")
local g = neotmux.group("plugin.tmux.status")

local M = {}

-- herdr's palette, which is Catppuccin Mocha. Every field is read on the tick, so assigning to
-- one from your own init.lua takes effect.
M.colors = {
  bar = "#181825", -- panel_bg: the band everything else sits on
  accent = "#89b4fa", -- blue: the window being shown
  on_accent = "#181825",
  tab = "#313244", -- surface0: every other window
  tab_text = "#6c7086", -- overlay0
  named_text = "#7f849c", -- overlay1: a window with a name you gave it
  right_text = "#7f849c", -- overlay1
  dim = "#6c7086", -- overlay0: the separators and the ellipses
}

-- What goes on the right, in order: each a function answering its text, and `true` after it for
-- the accent, or nil to be left out this tick. Empty, as herdr ships it.
--
--   require('plugin.tmux.status').right = { function() return os.date('%H:%M') end }
M.right = {}
M.separator = " "

M.band = g.reserve("top", 1)

local MIN_TAB_WIDTH = 8
-- herdr's `+` and two scroll buttons are what a strip must still hold beside the right side;
-- the room is kept though the buttons are the mouse's and are not drawn here.
local MIN_TAB_STRIP_WIDTH = MIN_TAB_WIDTH + 3 + 3 * 2

local columns = segments.columns

-- A window is its name if you gave it one, else its position counted from 1 — herdr's
-- `tab_display_name` — and a zoomed one says so.
local function tabs()
  local list, current = {}, neotmux.window.current()
  for at, id in ipairs(neotmux.window.list()) do
    local name = neotmux.window.name(id)
    local label = name or tostring(at)
    local focused = id == current
    if focused and neotmux.pane.zoomed() then
      label = label .. " Z"
    end
    list[#list + 1] = {
      label = label,
      named = name ~= nil,
      focused = focused,
      width = math.max(columns(label) + 4, MIN_TAB_WIDTH),
    }
  end
  return list
end

-- The first tab to draw, so the focused one sits as near the middle as it can.
local function centered_scroll(focused, list, available)
  local best, best_distance = focused, math.huge
  for start = 1, focused do
    local before = 0
    for i = start, focused - 1 do
      before = before + list[i].width + 1
    end
    if before < available then
      local center = before * 2 + math.min(list[focused].width, available - before)
      local distance = math.abs(center - available)
      if distance <= best_distance then
        best, best_distance = start, distance
      end
    end
  end
  return best
end

-- The latest a strip can start and still show its last tab whole.
local function max_scroll(list, available)
  if #list == 0 then
    return 1
  end
  local start, used = #list, list[#list].width
  for i = #list - 1, 1, -1 do
    local required = used + 1 + list[i].width
    if required > available then
      break
    end
    used, start = required, i
  end
  return start
end

local function right_runs()
  local runs = {}
  for _, entry in ipairs(M.right) do
    local text, accent = entry()
    if text and text ~= "" then
      if #runs > 0 and M.separator ~= "" then
        runs[#runs + 1] = { text = M.separator, fg = M.colors.dim, bg = M.colors.bar }
      end
      runs[#runs + 1] = accent
          and { text = text, fg = M.colors.on_accent, bg = M.colors.accent, bold = true }
        or { text = text, fg = M.colors.right_text, bg = M.colors.bar }
    end
  end
  return runs
end

local function tab_style(tab)
  if tab.focused then
    return M.colors.on_accent, M.colors.accent, tab.named
  end
  return tab.named and M.colors.named_text or M.colors.tab_text, M.colors.tab, false
end

g.on_tick(function(ctx)
  local row = ctx:reserved()
  -- Nil while the terminal is too small to spare the row. The panes have it back, so there
  -- is nowhere to draw and nothing to draw over.
  if not row then
    return
  end
  ctx:rect(nil, row.x, row.y, row.w, row.h, { bg = M.colors.bar })

  local strip = row.w
  local right = right_runs()
  local right_width = segments.width(right)
  if right_width > 0 and row.w - (right_width + 1) >= MIN_TAB_STRIP_WIDTH then
    segments.draw(ctx, row.x + row.w - right_width, row.y, right_width, right)
    strip = row.w - (right_width + 1)
  end

  local list = tabs()
  local total = #list > 0 and #list - 1 or 0
  for _, tab in ipairs(list) do
    total = total + tab.width
  end
  local first = 1
  if total > strip then
    for i, tab in ipairs(list) do
      if tab.focused then
        first = math.min(centered_scroll(i, list, strip), max_scroll(list, strip))
      end
    end
  end

  -- An ellipsis takes the colour of the tab it lands on, as herdr's leaves the cell's own.
  local x, stop, last, first_bg, last_bg = row.x, row.x + strip, nil, nil, nil
  for i = first, #list do
    local tab = list[i]
    local width = math.min(tab.width, stop - x)
    if width <= 0 then
      break
    end
    local padding = width - columns(tab.label)
    local left = padding // 2
    local fg, bg, bold = tab_style(tab)
    segments.draw(ctx, x, row.y, width, {
      {
        text = (" "):rep(left) .. tab.label .. (" "):rep(math.max(padding - left, 0)),
        fg = fg,
        bg = bg,
        bold = bold,
      },
    })
    last, last_bg = i, bg
    first_bg = first_bg or bg
    x = x + width + 1
    if width < tab.width then
      break
    end
  end

  if first > 1 then
    ctx:text(nil, row.x, row.y, "…", { fg = M.colors.dim, bg = first_bg })
  end
  if last and last < #list then
    ctx:text(nil, stop - 1, row.y, "…", { fg = M.colors.dim, bg = last_bg })
  end
end)

return M

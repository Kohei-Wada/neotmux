-- The notifications, drawn.
--
-- `neotmux.notify` raises one and the host holds it: it expires them on its own clock and
-- keeps the transcript a client attaching is told. What it does *not* decide any more is what
-- one looks like — that is this file, and it is the last of the visual surfaces to become a
-- plugin's, after the status bar, the borders, the sidebar and the footer.
--
-- One notification is one box. Two of them stack up from the bottom-right corner with a row
-- between, because two things raised by two plugins at two moments are two things, and a
-- single frame round them reads as one.
--
--   C-b C       take them all down
--
-- Nothing here is a host feature. `neotmux.notifications.list()` is the surface, `ctx:area()`
-- is the corner to put them in — the pane area, so a sidebar or a footer moves them rather
-- than being drawn over — and `neotmux.width` is what keeps the lines inside the frame.
local box = require("neotmux.box")
local g = neotmux.group("plugin.notify")

local M = {}

M.colors = {
  frame = "#181825", -- mantle
  tag = "#585b70", -- surface2: who raised it, quieter than what it says
  info = "#89b4fa", -- blue
  warn = "#f9e2af", -- yellow
  error = "#f38ba8", -- red
}

-- The most one takes of the surface, in columns and rows, and how many are shown at once. A
-- notification is an interruption, and one that covers what you were reading is a worse
-- interruption.
M.width = 48
M.rows = 4
M.stack = 3

-- The blank row between two boxes. Without it they share an edge and read as one window.
M.gap = 1

M.band = nil

local function palette(level)
  return M.colors[level] or M.colors.info
end

-- One notification as the lines it takes: the text cut to the width, in columns, and broken
-- rather than truncated where it does not fit.
local function wrap(text, width)
  local out = {}
  local line = ""
  for _, code in utf8.codes(text) do
    local ch = utf8.char(code)
    if neotmux.width(line .. ch) > width then
      out[#out + 1] = line
      line = ch
    else
      line = line .. ch
    end
    if #out >= M.rows then
      return out
    end
  end
  out[#out + 1] = line
  return out
end

-- One box, on the surface, at a place this file worked out. Not `neotmux.box`: that draws
-- into a float's own coordinates, and a notification is a corner of the surface that only
-- exists while something is showing — not a claim on a handle and a place.
local function draw(ctx, x, y, w, h)
  ctx:rect(nil, x, y, w, h, { bg = M.colors.frame })
  local glyphs = box.glyphs
  local across = string.rep(glyphs.horizontal, w - 2)
  local edge = { fg = box.colors.line, bg = M.colors.frame }
  ctx:text(nil, x, y, glyphs.top_left .. across .. glyphs.top_right, edge)
  ctx:text(nil, x, y + h - 1, glyphs.bottom_left .. across .. glyphs.bottom_right, edge)
  for row = 1, h - 2 do
    ctx:text(nil, x, y + row, glyphs.vertical, edge)
    ctx:text(nil, x + w - 1, y + row, glyphs.vertical, edge)
  end
end

if package.loaded["plugin.tmux.prefix"] then
  require("plugin.tmux.prefix").commands["C"] = function()
    neotmux.notifications.clear()
  end
  require("plugin.tmux.prefix").descriptions["C"] = "clear the notifications"
end

g.on_tick(function(ctx)
  local list = neotmux.notifications.list()
  if #list == 0 then
    return
  end
  local area = ctx:area()
  local width = math.min(M.width, area.w)
  local x = area.x + area.w - width
  -- Newest first, as the host hands them over, and the newest drawn lowest: a new one appears
  -- where your eye already is and pushes the older ones up.
  local bottom = area.y + area.h
  for at, one in ipairs(list) do
    if at > M.stack then
      break
    end
    local prefix = one.tag ~= "" and (one.tag .. ": ") or ""
    local lines = wrap(prefix .. one.text, width - 4)
    local h = #lines + 2
    local y = bottom - h
    if y < area.y or x < area.x then
      return
    end
    draw(ctx, x, y, width, h)
    for row, text in ipairs(lines) do
      ctx:text(nil, x + 2, y + row, text, { fg = palette(one.level), bg = M.colors.frame })
    end
    bottom = y - M.gap
  end
end)

return M

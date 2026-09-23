-- The notifications, drawn.
--
-- `neotmux.notify` raises one and the host holds it: it expires them on its own clock and
-- keeps the transcript that is replayed on stderr when a client leaves. What it does *not*
-- decide any more is what one looks like — that is this file, and it is the last of the
-- visual surfaces to become a plugin's, after the status bar, the borders, the sidebar and
-- the footer.
--
-- It used to be drawn by the host, over the bottom-right corner, one row each with no
-- background and the width counted in characters rather than columns — so a notification in
-- Japanese lost characters off the end and every one of them sat unreadably on top of whatever
-- the pane was printing.
--
--   C-b N       take them all down
--
-- Nothing here is a host feature. `neotmux.notifications.list()` is the surface,
-- `neotmux.box` is the frame round it, and `neotmux.width` is what keeps the lines inside it.
local box = require("neotmux.box")

local M = {}

M.colors = {
  frame = "#181825", -- mantle
  tag = "#585b70", -- surface2: who raised it, quieter than what it says
  info = "#89b4fa", -- blue
  warn = "#f9e2af", -- yellow
  error = "#f38ba8", -- red
}

-- The most it will take of the surface, in columns and rows. A notification is an
-- interruption, and one that covers what you were reading is a worse interruption.
M.width = 48
M.rows = 6

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

if package.loaded["plugin.prefix"] then
  require("plugin.prefix").commands["N"] = function()
    neotmux.notifications.clear()
  end
end

neotmux.on_tick(function(ctx)
  local list = neotmux.notifications.list()
  if #list == 0 then
    return
  end
  local size = ctx:size()
  local width = math.min(M.width, size.cols)

  -- Newest first, as the host hands them over, and drawn from the bottom up so a new one
  -- appears where your eye already is.
  local rows = {}
  for _, one in ipairs(list) do
    local prefix = one.tag ~= "" and (one.tag .. ": ") or ""
    for _, line in ipairs(wrap(prefix .. one.text, width - 4)) do
      rows[#rows + 1] = { text = line, level = one.level }
    end
    if #rows >= M.rows then
      break
    end
  end
  while #rows > M.rows do
    table.remove(rows)
  end

  local h = #rows + 2
  local x = size.cols - width
  local y = size.rows - h
  if x < 0 or y < 0 then
    return
  end

  -- Not a float: a float is a claim on a handle and a place, and this is a corner of the
  -- surface that only exists while something is showing. `ctx:rect` and `ctx:text` against the
  -- surface are the whole of it.
  ctx:rect(nil, x, y, width, h, { bg = M.colors.frame })
  local g = box.glyphs
  local across = string.rep(g.horizontal, width - 2)
  ctx:text(nil, x, y, g.top_left .. across .. g.top_right, {
    fg = box.colors.line,
    bg = M.colors.frame,
  })
  ctx:text(nil, x, y + h - 1, g.bottom_left .. across .. g.bottom_right, {
    fg = box.colors.line,
    bg = M.colors.frame,
  })
  for at, row in ipairs(rows) do
    local line = y + at
    ctx:text(nil, x, line, g.vertical, { fg = box.colors.line, bg = M.colors.frame })
    ctx:text(nil, x + width - 1, line, g.vertical, { fg = box.colors.line, bg = M.colors.frame })
    ctx:text(nil, x + 2, line, row.text, { fg = palette(row.level), bg = M.colors.frame })
  end
end)

return M

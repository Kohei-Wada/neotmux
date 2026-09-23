-- A frame around a float, with corners that are not square.
--
-- A float takes no cells and draws nothing of its own: the host gives you a rect and a
-- coordinate space, and everything inside it is yours. So a window that reads as a window —
-- one with a line round it and its content held off the edge — is a convention, and this is
-- the file that holds it, in the same spirit as `neotmux.mode`. The host has never heard of it.
--
--   local box = require('neotmux.box')
--   local inner = box.draw(ctx, win, rect, { title = 'sessions' })
--   ctx:text(win, inner.x, inner.y, 'the first row of your content')
--
-- `draw` fills the rect, puts the frame on its edge, and hands back the rect *inside* the
-- frame, so a caller never does the arithmetic twice. A rect too small to hold a frame — under
-- two cells either way — is filled and handed back whole rather than drawn over.
local M = {}

-- Rounded, because a box drawn in a terminal is a box somebody chose the corners of, and these
-- are the ones that do not read as a table. Every glyph here is one column wide; a heavier set
-- would want checking against `neotmux.width` before it went in.
M.glyphs = {
  top_left = "\u{256d}", -- ╭
  top_right = "\u{256e}", -- ╮
  bottom_left = "\u{2570}", -- ╰
  bottom_right = "\u{256f}", -- ╯
  horizontal = "\u{2500}", -- ─
  vertical = "\u{2502}", -- │
}

-- Catppuccin Mocha, the palette everything else neotmux ships is drawn from. Read on every
-- call, so assigning to one from your own `init.lua` takes effect.
M.colors = {
  frame = "#181825", -- mantle: the ground the window sits on
  line = "#45475a", -- surface1: present without being the thing you look at
  title = "#cba6f7", -- mauve
}

-- The frame, and the rect inside it. `opts.title` is drawn into the top line where there is
-- room for it; `opts.bg` and `opts.line` override the colours for this one call.
function M.draw(ctx, win, rect, opts)
  opts = opts or {}
  local bg = opts.bg or M.colors.frame
  local line = opts.line or M.colors.line
  ctx:rect(win, 0, 0, rect.w, rect.h, { bg = bg })
  if rect.w < 2 or rect.h < 2 then
    return { x = 0, y = 0, w = rect.w, h = rect.h }
  end

  local g = M.glyphs
  local across = string.rep(g.horizontal, rect.w - 2)
  ctx:text(win, 0, 0, g.top_left .. across .. g.top_right, { fg = line, bg = bg })
  ctx:text(win, 0, rect.h - 1, g.bottom_left .. across .. g.bottom_right, { fg = line, bg = bg })
  for row = 1, rect.h - 2 do
    ctx:text(win, 0, row, g.vertical, { fg = line, bg = bg })
    ctx:text(win, rect.w - 1, row, g.vertical, { fg = line, bg = bg })
  end

  -- The title sits in the top line rather than above it, with a space either side so the rule
  -- does not run into the letters. Dropped rather than cut where it will not fit: half a word
  -- in a frame reads as a bug, and the window is the caller's to make wider.
  if opts.title then
    local text = " " .. opts.title .. " "
    if neotmux.width(text) <= rect.w - 4 then
      ctx:text(win, 2, 0, text, { fg = opts.title_fg or M.colors.title, bg = bg })
    end
  end

  return { x = 1, y = 1, w = rect.w - 2, h = rect.h - 2 }
end

return M

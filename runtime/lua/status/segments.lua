-- A run of text in one pair of colours, and how a row of them lands on the surface.
--
-- Columns are counted with `utf8.len` and never with `#`: `#s` is a byte count, and every
-- cap and icon on this bar is a three-byte glyph, so a `#` here puts every run after it in
-- the wrong column.
local M = {}

-- Bytes are the honest answer for a string that is not UTF-8 at all — a working directory
-- read out of /proc need not be.
function M.columns(text)
  return utf8.len(text) or #text
end

function M.width(runs)
  local total = 0
  for _, run in ipairs(runs) do
    total = total + M.columns(run.text)
  end
  return total
end

local function clip(text, cols)
  if not utf8.len(text) then
    return text:sub(1, cols)
  end
  local stop = utf8.offset(text, cols + 1)
  return stop and text:sub(1, stop - 1) or text
end

-- Draws left to right from `x`, within `limit` columns. A run that only partly fits is cut
-- rather than allowed to run past the band into somebody else's cells.
function M.draw(ctx, x, y, limit, runs)
  local at = x
  for _, run in ipairs(runs) do
    local room = limit - (at - x)
    if room <= 0 then
      break
    end
    local text = run.text
    if M.columns(text) > room then
      text = clip(text, room)
    end
    if text ~= "" then
      ctx:text(nil, at, y, text, { fg = run.fg, bg = run.bg })
      at = at + M.columns(text)
    end
  end
  return at
end

return M

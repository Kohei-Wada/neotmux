-- A run of text in one pair of colours, and how a row of them lands on the surface.
--
-- Columns are counted with `neotmux.width`, the renderer's own measure, and never with `#` or
-- `utf8.len`: every cap and icon on this bar is a three-byte glyph, and `あ` is one character
-- that takes two columns, so either count puts every run after it in the wrong column.
local M = {}

-- Bytes are the honest answer for a string that is not UTF-8 at all — a working directory
-- read out of /proc need not be — and `neotmux.width` will not take one.
function M.columns(text)
  if not utf8.len(text) then
    return #text
  end
  return neotmux.width(text)
end

function M.width(runs)
  local total = 0
  for _, run in ipairs(runs) do
    total = total + M.columns(run.text)
  end
  return total
end

-- `text` cut to at most `cols` columns, on a character boundary: a wide character that would
-- straddle the edge is left out rather than drawn half.
function M.clip(text, cols)
  if not utf8.len(text) then
    return text:sub(1, cols)
  end
  local used = 0
  for at, code in utf8.codes(text) do
    used = used + neotmux.width(utf8.char(code))
    if used > cols then
      return text:sub(1, at - 1)
    end
  end
  return text
end

-- Draws left to right from `x`, within `limit` columns. A run that only partly fits is cut
-- rather than allowed to run past the band into somebody else's cells. `target` is what
-- `ctx:text` draws on — nil for the surface, or a float — and `bg` is the colour a run with none
-- of its own is drawn on.
function M.draw(ctx, x, y, limit, runs, target, bg)
  local at = x
  for _, run in ipairs(runs) do
    local room = limit - (at - x)
    if room <= 0 then
      break
    end
    local text = run.text or ""
    if M.columns(text) > room then
      text = M.clip(text, room)
    end
    if text ~= "" then
      ctx:text(target, at, y, text, { fg = run.fg, bg = run.bg or bg, bold = run.bold })
      at = at + M.columns(text)
    end
  end
  return at
end

return M

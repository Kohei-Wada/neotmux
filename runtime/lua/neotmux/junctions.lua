-- Where divider segments meet, and which way the arms run.
--
-- Geometry and nothing else, the way `neotmux.nav` is: it is handed the segments a caller
-- has already worked out and answers with cells and arms. It draws nothing, knows no glyph,
-- and never asks the host anything — which glyph an arm set deserves is the caller's
-- opinion, and box drawing is only one of the answers.
--
-- A segment is `{ x1, y1, x2, y2 }` with one axis held still, its ends inclusive. A cell has
-- an arm in a direction when a segment covers the neighbouring cell that way: that is all
-- "the arms that meet here" can mean to a grid, and it stays true whichever pane drew which
-- segment. Fewer than two arms is a line passing through, not a junction.
local M = {}

local function key(x, y)
  return x .. "," .. y
end

-- Every cell the segments cover, so a neighbour is one lookup rather than a scan.
local function covered(segments)
  local cells = {}
  for _, s in ipairs(segments) do
    for y = s.y1, s.y2 do
      for x = s.x1, s.x2 do
        cells[key(x, y)] = true
      end
    end
  end
  return cells
end

local function sorted_keys(set)
  local out = {}
  for value in pairs(set) do
    out[#out + 1] = value
  end
  table.sort(out)
  return out
end

-- The cells where a vertical segment's column crosses a horizontal segment's row, each with
-- `x`, `y` and the four arms as booleans. A crossing cell may itself be covered — a divider
-- running past a neighbour's is where the tees come from — so occupancy of the cell says
-- nothing, only its neighbours do.
function M.of(segments)
  local cells = covered(segments)
  local columns, rows = {}, {}
  for _, s in ipairs(segments) do
    -- A one-cell segment names no axis, so it offers no crossing of its own; it still
    -- covers its cell, and so can still be somebody else's arm.
    if s.y1 < s.y2 then
      columns[s.x1] = true
    elseif s.x1 < s.x2 then
      rows[s.y1] = true
    end
  end

  local found = {}
  for _, y in ipairs(sorted_keys(rows)) do
    for _, x in ipairs(sorted_keys(columns)) do
      local arms = {
        x = x,
        y = y,
        up = cells[key(x, y - 1)] or false,
        down = cells[key(x, y + 1)] or false,
        left = cells[key(x - 1, y)] or false,
        right = cells[key(x + 1, y)] or false,
      }
      local count = 0
      for _, dir in ipairs({ "up", "down", "left", "right" }) do
        count = count + (arms[dir] and 1 or 0)
      end
      if count >= 2 then
        found[#found + 1] = arms
      end
    end
  end
  return found
end

return M

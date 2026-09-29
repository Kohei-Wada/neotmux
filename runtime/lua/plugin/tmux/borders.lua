-- The dividers between panes.
--
-- The host reserves the space; this plugin draws in it. `neotmux.opt.pane_gutter` says
-- "leave this many cells between panes", the same deal as `neotmux.reserve` saying "leave a
-- row": the host takes the cells away from the panes and never learns what goes in them. So
-- a divider costs no pane a column of its shell's output.
--
-- Which panes touch still never has to be asked for. With a gutter of g, a pane's right
-- neighbour starts at `r.x + r.w + g`, so the g columns from `r.x + r.w` are the gap between
-- the two — and a pane whose right edge is not the rightmost has somebody on the other side
-- of that gap. Every divider is drawn once, by the pane on its left or above.
--
-- The far edges are the panes' own, not `ctx:size()`: a band reserved at the top of the
-- surface moves the panes down, so the pane area is where the panes are and nowhere else.
--
-- Where two dividers cross, the cell they cross in belongs to neither pane's extent, so no
-- pane draws it and the cross has a hole. `neotmux.junctions` works out which arms meet in
-- such a cell; this file only decides what to put there, and draws it after the segments so
-- the junction wins the cell.
local junctions = require("neotmux.junctions")
local g = neotmux.group("plugin.tmux.borders")

local GUTTER = 1

-- Every field is read on the tick, so assigning to one from your own init.lua takes effect.
-- The glyphs are as thin as a terminal draws: a cell cannot be subdivided, so one cell of
-- box drawing is the whole budget. A tee is named for the arm that is on its own: `├` is
-- `tee_right`, because right is the way its stem points.
local M = {
  fg = 8,
  vertical = "│",
  horizontal = "─",
  cross = "┼",
  tee_up = "┴",
  tee_down = "┬",
  tee_left = "┤",
  tee_right = "├",
}

-- What belongs in a cell the given arms meet in, or nil for an arm set that is no junction:
-- two arms in line are the divider carrying on, and an elbow is a shape these dividers
-- cannot make, since every one of them runs from one side of the pane area to the other.
local function glyph_for(arms)
  local up, down, left, right = arms.up, arms.down, arms.left, arms.right
  if up and down and left and right then
    return M.cross
  elseif up and down and right then
    return M.tee_right
  elseif up and down and left then
    return M.tee_left
  elseif left and right and down then
    return M.tee_down
  elseif left and right and up then
    return M.tee_up
  end
  return nil
end

neotmux.opt.pane_gutter = GUTTER

-- The dividers and where they cross, for the panes as they were last laid out. They follow
-- from where the panes are and nothing else, so a tick with the panes where they were draws
-- them again without working them out again.
local last = { key = nil, segments = {}, crossings = {} }

local function lay_out(outers)
  local right, bottom = 0, 0
  for _, r in ipairs(outers) do
    right = math.max(right, r.x + r.w)
    bottom = math.max(bottom, r.y + r.h)
  end
  local segments = {}
  for _, r in ipairs(outers) do
    -- A pane the terminal has shrunk to nothing has no neighbour worth a divider.
    if r.w > 0 and r.h > 0 then
      if r.x + r.w < right then
        segments[#segments + 1] =
          { x1 = r.x + r.w, y1 = r.y, x2 = r.x + r.w, y2 = r.y + r.h - 1, vertical = true }
      end
      if r.y + r.h < bottom then
        segments[#segments + 1] =
          { x1 = r.x, y1 = r.y + r.h, x2 = r.x + r.w - 1, y2 = r.y + r.h, vertical = false }
      end
    end
  end
  return segments, junctions.of(segments)
end

g.on_tick(function(ctx)
  local outers, key = {}, {}
  for _, id in ipairs(neotmux.pane.list()) do
    local r = ctx:pane_outer(id)
    if r then
      outers[#outers + 1] = r
      key[#key + 1] = r.x .. " " .. r.y .. " " .. r.w .. " " .. r.h
    end
  end
  key = table.concat(key, ",")
  if key ~= last.key then
    last.key = key
    last.segments, last.crossings = lay_out(outers)
  end

  -- The glyphs and the colour are read here, every tick, so assigning to one takes effect.
  local colours = { fg = M.fg }
  for _, s in ipairs(last.segments) do
    ctx:line(nil, s.x1, s.y1, s.x2, s.y2, colours, s.vertical and M.vertical or M.horizontal)
  end
  -- A junction is a single cell, which `line` draws as a run of one.
  for _, arms in ipairs(last.crossings) do
    local glyph = glyph_for(arms)
    if glyph then
      ctx:line(nil, arms.x, arms.y, arms.x, arms.y, colours, glyph)
    end
  end
end)

return M

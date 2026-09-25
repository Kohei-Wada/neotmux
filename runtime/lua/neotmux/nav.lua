-- Which pane is that way.
--
-- Geometry and nothing else: it reads `neotmux.pane.rect`, which answers outside a tick, and
-- returns an id. What to do about that id is the caller's business, so this file has no
-- opinion about keys and registers nothing.
--
-- "That way" is not "the previous entry in `pane.list()`". The panes on that side of you are
-- the ones whose facing edge is nearest yours; the one you meant is whichever of those faces
-- most of your own edge. That is what `select-pane -L` means, and it is the only rule that
-- survives nested splits: a tall pane beside a column of short ones has several candidates to
-- its right, and the one a finger would land on is the one it shares the most rows with.
local M = {}

-- How much of `a0..a1` and `b0..b1` is shared. Zero or less means the two do not face each
-- other at all, whatever their edges do.
local function shared(a0, a1, b0, b1)
  return math.min(a1, b1) - math.max(a0, b0)
end

-- How far `it` is on that side of `me` — negative when it is not on that side — and how much
-- of the facing edge the two have in common. The distance is not tested for zero because of
-- the gutter: the host takes those cells off both panes, so touching neighbours are exactly
-- `pane_gutter` apart and the nearest edge is the touching one at any gutter.
local FACING = {
  left = function(me, it)
    return me.x - (it.x + it.w), shared(me.y, me.y + me.h, it.y, it.y + it.h)
  end,
  right = function(me, it)
    return it.x - (me.x + me.w), shared(me.y, me.y + me.h, it.y, it.y + it.h)
  end,
  up = function(me, it)
    return me.y - (it.y + it.h), shared(me.x, me.x + me.w, it.x, it.x + it.w)
  end,
  down = function(me, it)
    return it.y - (me.y + me.h), shared(me.x, me.x + me.w, it.x, it.x + it.w)
  end,
}

-- The pane in `dir` of the focused one, or nil when there is none — the edge of the window,
-- or a session with nothing focused.
function M.toward(dir)
  local facing = FACING[dir] or error("unknown direction '" .. tostring(dir) .. "'")
  local me = neotmux.pane.current()
  local mine = me and neotmux.pane.rect(me)
  if not mine then
    return nil
  end

  local best, best_gap, best_faced
  for _, id in ipairs(neotmux.pane.list()) do
    local it = id ~= me and neotmux.pane.rect(id)
    if it then
      local gap, faced = facing(mine, it)
      if
        gap >= 0
        and faced > 0
        and (not best or gap < best_gap or (gap == best_gap and faced > best_faced))
      then
        best, best_gap, best_faced = id, gap, faced
      end
    end
  end
  return best
end

return M

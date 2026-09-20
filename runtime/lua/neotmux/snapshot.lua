-- A pane's history and its visible screen, read once and then never again.
--
-- Not a cache. The reason to take the lines in one pass is not that reading is expensive —
-- though it is, every row being built character by character out of the terminal's cells —
-- but that the child keeps printing while somebody is reading. A buffer re-read every frame
-- is a buffer whose line 400 is one thing when a cursor lands on it and another thing when it
-- is yanked, and the whole point of a copy mode is that what you took is what you saw. tmux
-- clones the grid on the way in for exactly this reason and reads only the clone.
--
-- The host has no way to freeze a pane, so a snapshot is the nearest honest thing: what the
-- pane does after it is taken is not in here, and a mode reading it stays coherent by not
-- changing. When a freeze does arrive the caller changes nothing — it is already reading a
-- fixed buffer.
--
-- It registers nothing and draws nothing. Line 0 is the oldest line the pane held at the
-- moment it was taken, which is the numbering `pane.scrollback` already uses, and the visible
-- screen is the last rows of the same run.
local M = {}

-- The whole buffer, or nil for a pane that is not there. One `scrollback` call and one
-- `content` call, both outside any tick: this runs from the key that enters the mode, where
-- there is no frame to lose, and it is the only read the mode ever makes.
function M.take(pane)
  local history = neotmux.pane.history(pane)
  if history == nil then
    return nil
  end
  local lines = {}
  if history > 0 then
    -- No count: the whole history, which is what "read it once" means.
    local above = neotmux.pane.scrollback(pane)
    if above == nil then
      return nil
    end
    table.move(above, 1, #above, 1, lines)
  end
  local screen = neotmux.pane.content(pane)
  if screen == nil then
    return nil
  end
  table.move(screen, 1, #screen, #lines + 1, lines)
  return {
    pane = pane,
    lines = lines,
    -- Where the visible screen started when the snapshot was taken. The one number that
    -- relates this buffer back to what the pane is showing.
    history = history,
    total = #lines,
    -- What the pane's terminal was at. A mode can ask whether the pane has been fed since,
    -- which is the difference between "the pane's own grid still shows the bottom of this
    -- buffer" and "it does not".
    revision = neotmux.pane.revision(pane),
  }
end

-- Line `n` of the buffer, zero-based, or nil past either end.
function M.line(snapshot, n)
  return snapshot.lines[n + 1]
end

-- The `count` lines from `n`, zero-based, short at the end. Nothing is built here: the lines
-- already exist, and this is a slice of them.
function M.window(snapshot, n, count)
  local out = {}
  for at = n, math.min(n + count, snapshot.total) - 1 do
    out[#out + 1] = snapshot.lines[at + 1]
  end
  return out
end

return M

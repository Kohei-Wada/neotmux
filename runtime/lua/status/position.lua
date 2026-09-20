-- Where a thing sits in a list, as a person counts.
--
-- A `PaneId` and a `WindowId` are allocated once and never reused, so the third pane of a
-- session that has opened and closed two is id 5. That is right for identity and wrong for a
-- bar: what a person reads off it is a position, which closes the gap when a neighbour goes.
local M = {}

-- The 1-based position of `id` in `ids`, or nil when it is not in the list.
function M.of(ids, id)
  for index, candidate in ipairs(ids) do
    if candidate == id then
      return index
    end
  end
  return nil
end

return M

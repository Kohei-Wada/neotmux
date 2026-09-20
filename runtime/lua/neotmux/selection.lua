-- A region between two points of a line buffer: which cells of a row it covers, and what
-- text it holds.
--
-- Arithmetic and nothing else. It calls no API, registers nothing and never sees a pane — the
-- caller hands it two points and a way of reading a line, and gets back spans to draw and a
-- string. That is what makes both halves testable without a terminal underneath them.
--
-- A point is `{ line = , col = }`: an **absolute** line number in the caller's own buffer and
-- a zero-based column. Absolute and never a screen row — that is the whole design. A
-- selection held in screen coordinates evaporates the moment the view scrolls under it; held
-- against the buffer it survives scrolling, and the projection back onto the screen is
-- `M.span`, asked once per visible row.
local M = {}

local function before(a, b)
  if a.line ~= b.line then
    return a.line < b.line
  end
  return a.col < b.col
end

-- The two ends in reading order, whichever way round the caller holds them. A selection is
-- dragged as often upwards as downwards, and every question below is asked of the earlier end
-- first.
function M.range(a, b)
  if before(b, a) then
    return b, a
  end
  return a, b
end

-- The cells of `line` the region covers, as a zero-based `x` and a width, or nil for a line
-- it does not reach. A line in the middle of the region runs to the right edge: the region is
-- the text between two points and not a rectangle, and the run of cells past the end of a
-- short line is what selecting through it looks like.
--
-- This is also where an end that is off the screen is dealt with, and it needs no special
-- case: a row below the first line starts at column 0 and a row above the last one ends at
-- the right edge, so an end scrolled out of view is clamped rather than lost, and a view with
-- both ends off the same side of it simply gets nil for every row. Hidden, never deleted —
-- scroll back and the selection is still the two points it always was.
function M.span(a, b, line, width)
  local first, last = M.range(a, b)
  if line < first.line or line > last.line then
    return nil
  end
  local x = line == first.line and first.col or 0
  local right = line == last.line and last.col or width - 1
  right = math.min(right, width - 1)
  if right < x then
    return nil
  end
  return x, right - x + 1
end

-- The characters of `s` from `from` to `to`, both zero-based and inclusive; a nil `to` runs to
-- the end.
--
-- Characters and not bytes. Not columns either, and on a line holding a double-width
-- character the two part company: the host hands a wide character back once, at the column it
-- starts in, so character 5 of such a line is not column 5 and a slice of it drifts left by
-- one per wide character before it. Every cell this file is asked about elsewhere is a column,
-- because `ctx:tint` counts columns. Closing that gap needs a way to measure a string in
-- columns, which this ABI does not have; until it does, an ASCII line is exact and a CJK one
-- can take a character too few from the right.
function M.slice(s, from, to)
  local first = utf8.offset(s, from + 1)
  if first == nil then
    return ""
  end
  local last = to and utf8.offset(s, to + 2)
  return last and s:sub(first, last - 1) or s:sub(first)
end

-- The text of the region, one entry per line, joined with newlines. `read(n)` answers line
-- `n` of the caller's buffer, or nothing for a line it does not have.
--
-- A row whose selection starts past the end of its line contributes an empty string rather
-- than a run of spaces: the host cuts a line's trailing blanks before the caller ever sees
-- it, and padding them back on would put whitespace in the register that was never typed.
--
-- **Every line gets a newline after it, including one that was only soft-wrapped.** That is
-- wrong and it is knowingly wrong. A terminal wraps a long command across two rows and the
-- right answer is to join them with nothing at all, which is the rule tmux keeps in
-- `window_copy_copy_line`; but it can only keep it because its grid says which rows are
-- continuations. This ABI hands back lines and nothing else — there is no wrap flag on a row
-- and no way to infer one, since a wrapped row and a row that happens to fill the width read
-- identically once trailing blanks are cut. Guessing would put the defect somewhere
-- unpredictable instead of somewhere stated, so the newline goes in every time and a yanked
-- wrapped command line has a break in the middle of it. It wants a `wrapped` flag on the
-- lines the host returns.
function M.text(a, b, read)
  local first, last = M.range(a, b)
  local out = {}
  for line = first.line, last.line do
    local s = read(line) or ""
    local from = line == first.line and first.col or 0
    local to = line == last.line and last.col or nil
    out[#out + 1] = M.slice(s, from, to)
  end
  return table.concat(out, "\n")
end

return M

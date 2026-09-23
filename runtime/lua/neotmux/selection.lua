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

-- The characters of `s` under columns `from` to `to`, both zero-based and inclusive; a nil
-- `to` runs to the end of the line.
--
-- **Columns, not characters.** Every other number this file deals in is a column, because
-- `ctx:tint` counts columns and so do the cursor and the highlight drawn with it. A line
-- holding a double-width character is where the two part company: the host hands such a
-- character back once, at the column it starts in, so character 5 of that line is not column
-- 5. Reading these as character indices — which this did until issue #3 — yanks text that is
-- not the text the highlight covered.
--
-- `columns(ch)` is how one character is measured. Passed in rather than reached for, because
-- this module calls no API: the caller hands it a way of reading a line and a way of
-- measuring one, and does the host talking itself. `neotmux.width` is what copy mode passes.
--
-- A wide character the selection only half covers is in it: the column is highlighted, so
-- what sits under that column is selected. That holds at both ends. A zero-width character —
-- a combining mark — rides the cell before it and goes wherever that character went; one at
-- the start of a line has nothing to ride and is taken only when the slice starts at column
-- zero.
function M.slice(s, from, to, columns)
  local out = {}
  local col = 0
  local rode = from == 0
  for _, code in utf8.codes(s) do
    local ch = utf8.char(code)
    local width = columns(ch)
    local take
    if width == 0 then
      take = rode
    else
      if to ~= nil and col > to then
        break
      end
      take = col + width - 1 >= from
    end
    if take then
      out[#out + 1] = ch
    end
    rode = take
    col = col + width
  end
  return table.concat(out)
end

-- The text of the region, one entry per line, joined with newlines. `read(n)` answers line
-- `n` of the caller's buffer, or nothing for a line it does not have, and `columns(ch)`
-- measures one character — see [`M.slice`], which does the cutting.
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
function M.text(a, b, read, columns)
  local first, last = M.range(a, b)
  local out = {}
  for line = first.line, last.line do
    local s = read(line) or ""
    local from = line == first.line and first.col or 0
    local to = line == last.line and last.col or nil
    out[#out + 1] = M.slice(s, from, to, columns)
  end
  return table.concat(out, "\n")
end

return M

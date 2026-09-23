-- Where the words are on one line, counted in columns.
--
-- Columns, never bytes and never codepoints: a line out of a pane is what was drawn on a grid,
-- so a cursor at "column 12" is over the cell at 12 — and `あ` is one codepoint, three bytes
-- and two columns. `neotmux.width` is the host's own measure, the one the renderer used, so
-- asking it is what keeps a motion landing where the character is.
--
-- The classes are vim's, roughly: a run of word characters, a run of punctuation, or blank.
-- `w` and `b` step between runs; `e` lands on the last column of one.
local M = {}

-- What counts as a word character. Anything the pattern does not match, and is not a space, is
-- punctuation — so a path breaks at its slashes the way it does in vim.
M.word = "[%w_]"

local function columns(s)
  return neotmux.width(s)
end

-- The line as a list of one-cell entries: the column each character starts at, and its class.
-- Built once per motion, which is a line rather than a screen.
local function cells(line)
  local out = {}
  local col = 0
  for _, code in utf8.codes(line) do
    local ch = utf8.char(code)
    local class = "blank"
    if ch ~= " " and ch ~= "\t" then
      class = ch:match(M.word) and "word" or "punct"
    end
    local width = math.max(1, columns(ch))
    out[#out + 1] = { col = col, class = class, width = width }
    col = col + width
  end
  return out
end

-- The entry the column is inside, or the one after it where the column is past the end.
local function at(list, col)
  for index, entry in ipairs(list) do
    if col < entry.col + entry.width then
      return index
    end
  end
  return #list + 1
end

--- The column the next word starts at, or nil for "nothing left on this line".
function M.forward(line, col)
  local list = cells(line)
  local index = at(list, col)
  local from = list[index]
  -- Out of the run the cursor is in, then over any blanks.
  if from then
    local class = from.class
    while list[index] and list[index].class == class and class ~= "blank" do
      index = index + 1
    end
  end
  while list[index] and list[index].class == "blank" do
    index = index + 1
  end
  return list[index] and list[index].col
end

--- The column the word before the cursor starts at, or nil.
function M.back(line, col)
  local list = cells(line)
  local index = math.min(at(list, col), #list + 1) - 1
  while list[index] and list[index].class == "blank" do
    index = index - 1
  end
  if not list[index] then
    return nil
  end
  local class = list[index].class
  while list[index - 1] and list[index - 1].class == class do
    index = index - 1
  end
  return list[index].col
end

--- The column the current or next word ends at, or nil.
function M.word_end(line, col)
  local list = cells(line)
  local index = at(list, col) + 1
  while list[index] and list[index].class == "blank" do
    index = index + 1
  end
  if not list[index] then
    return nil
  end
  local class = list[index].class
  while list[index + 1] and list[index + 1].class == class do
    index = index + 1
  end
  return list[index].col
end

--- The column of the first character that is not blank, or 0 for a line that is all blank.
function M.first_word(line)
  local list = cells(line)
  for _, entry in ipairs(list) do
    if entry.class ~= "blank" then
      return entry.col
    end
  end
  return 0
end

--- The column of the last character that is not blank, or 0.
function M.last(line)
  local list = cells(line)
  for index = #list, 1, -1 do
    if list[index].class ~= "blank" then
      return list[index].col
    end
  end
  return 0
end

return M

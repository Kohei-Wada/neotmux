-- The keys that edit one line of text, the same wherever a line is typed: a prompt, a list's
-- query, the question a list asks. Each of those used to keep its own few, and no two kept the
-- same ones.
--
--   local line = require('neotmux.line')
--   local value = line.edit(value, key)          -- nil: not a key that edits a line
--   local value = line.edit(value, key, accept)  -- accept(key, value): may this character join?
--
--   Backspace, C-h   the character before the end (a terminal's C-h arrives as Backspace)
--   C-w              the word before the end, and the blanks after it, as a shell's does
--   C-u              everything
--   anything one character long is typed
--
-- A character is a character and not a byte: taking one back off `作業` leaves `作`.
local M = {}

local function without_last_character(value)
  local at = utf8.offset(value, -1)
  if at == nil then
    return value:sub(1, -2)
  end
  return value:sub(1, at - 1)
end

-- A shell's `unix-word-rubout`: the blanks at the end, then the run of anything else before them.
local function without_last_word(value)
  return (value:gsub("%s*$", ""):gsub("%S*$", ""))
end

-- The value after `key`, or nil when `key` is not one that edits a line — the caller's to
-- handle, or to let through.
function M.edit(value, key, accept)
  if key == "Backspace" or key == "C-h" then
    return without_last_character(value)
  elseif key == "C-w" then
    return without_last_word(value)
  elseif key == "C-u" then
    return ""
  elseif utf8.len(key) == 1 then
    if accept and not accept(key, value) then
      return value
    end
    return value .. key
  end
  return nil
end

return M

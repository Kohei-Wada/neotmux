-- What a working directory looks like on a status bar: `$HOME` written `~`, the way a shell
-- prompt writes it, and short enough for the space there is.
local segments = require("status.segments")

local M = {}

-- ASCII rather than `…`, whose width is ambiguous: a terminal set up for CJK draws it two
-- cells wide and it would overprint the component after it.
local ELIDED = "..."

-- A nil or empty home is the answer where the process had none, and costs a `~` and nothing
-- else: the path is written out in full.
function M.tilde(cwd, home)
  if not home or home == "" then
    return cwd
  end
  if cwd == home then
    return "~"
  end
  local rest = cwd:match("^" .. home:gsub("%W", "%%%0") .. "/(.*)$")
  return rest and ("~/" .. rest) or cwd
end

-- `nil` when not even the last component fits: a directory nobody can read is worse than no
-- directory at all, and the caller drops the module instead.
function M.shorten(cwd, home, cols)
  local text = M.tilde(cwd, home)
  if segments.columns(text) <= cols then
    return text
  end
  local rest = text
  while true do
    rest = rest:match("^[^/]*/(.+)$")
    if not rest then
      return nil
    end
    local elided = ELIDED .. "/" .. rest
    if segments.columns(elided) <= cols then
      return elided
    end
  end
end

return M

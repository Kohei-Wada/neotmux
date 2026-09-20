-- What a working directory looks like on a status bar: `$HOME` written `~`, the way a shell
-- prompt writes it, and short enough for the space there is.
local segments = require("status.segments")

local M = {}

-- ASCII rather than `…`, whose width is ambiguous: a terminal set up for CJK draws it two
-- cells wide and it would overprint the component after it.
local ELIDED = "..."

-- Where the host looks for Lua under a home directory. The VM has no `os.getenv`, so a layer
-- the host resolved out of `$HOME` is the only place `$HOME` reaches a plugin at all.
local UNDER_HOME = { "/%.config/neotmux/lua/%?%.lua", "/%.local/share/neotmux/lua/%?%.lua" }

-- The home directory, recovered from the search path. Nil when neither layer is on it, which
-- costs a `~` and nothing else; set `status.home` yourself if you want one anyway.
function M.home(search_path)
  -- The installed layer is reached as `<bin>/../share/neotmux`, and the host puts that on the
  -- path unnormalised, so the `..` is collapsed here before anything is matched against it.
  local flat = (search_path or ""):gsub("[^/;]+/%.%./", "")
  for _, layer in ipairs(UNDER_HOME) do
    local home = flat:match("([^;]*)" .. layer)
    if home and home ~= "" then
      return home
    end
  end
end

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

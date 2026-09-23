-- What a pane looks like it is doing: working, blocked, or idle.
--
-- The host has no such concept — it knows a pane's bytes and a stamp that moves when they
-- change, and everything above that is a guess about text on a screen. This is where the guess
-- lives, so that the plugin which *notifies* you and the plugin which *draws* you a list are
-- reading the same one.
--
--   local state = require('neotmux.state')
--   state.of(pane)        --> 'working' | 'blocked' | 'idle', or nil for a pane that went away
--   state.icon(state.of(pane))
--
-- The three words are herdr's, and so is the shape of the answer: an agent you are running is
-- either getting on with it, waiting for you, or doing nothing, and which of those it is, is
-- the thing you open a sidebar to find out.
local M = {}

-- Lua patterns, matched against the last `M.lines` non-blank lines of a pane. Replace the
-- table to replace the guess:
--
--   require('neotmux.state').patterns = { 'my agent is waiting' }
--
-- These are the prompt shapes that mean a program has stopped and wants an answer. They are a
-- guess about text on a screen and nothing more: a pane showing this repository's own source
-- matches several of them, and an agent that asks its question in words nobody listed here
-- matches none. Narrow the table to what you actually run.
M.patterns = {
  "%[[yYnN]/[yYnN]%]", -- [y/N]
  "%([yYnN]/[yYnN]%)", -- (y/n)
  "%[[yY]es/[nN]o%]",
  "[Dd]o you want to",
  "[Pp]ress %a+ to continue",
  "[Cc]ontinue%?",
  "[Pp]assword:",
  "[Pp]assphrase",
}

-- How many of the pane's last non-blank lines to match against. A prompt is the last thing a
-- program said, and matching the whole screen would fire on anything it ever printed.
M.lines = 5

-- How long after a pane last said something it still counts as working, in seconds. A shell
-- that printed a prompt and stopped is idle a moment later; an agent printing as it goes stays
-- working between its lines.
M.working_for = 2

M.icons = {
  working = "\u{25cf}", -- ●
  blocked = "\u{25c9}", -- ◉
  idle = "\u{25cb}", -- ○
}

-- Everything known about a pane, and when it was learned. Read on demand rather than on a tick
-- of its own: every caller is already in one, and a module that registered its own would sample
-- whether or not anybody was looking.
local seen = {}

local function matches(line)
  for _, pattern in ipairs(M.patterns) do
    if line:find(pattern) then
      return true
    end
  end
  return false
end

-- Whether the pane's last few lines read as a question, or nil when it went away.
local function asking(pane)
  local lines = neotmux.pane.content(pane)
  if lines == nil then
    return nil
  end
  local looked = 0
  for i = #lines, 1, -1 do
    if lines[i] ~= "" then
      if matches(lines[i]) then
        return true
      end
      looked = looked + 1
      if looked >= M.lines then
        break
      end
    end
  end
  return false
end

function M.forget(pane)
  seen[pane] = nil
end

-- The whole cost control: the lines are built only for a pane that has been fed since it was
-- last looked at, which on a session nobody is typing into is none of them. The stamp is one
-- call and no copying; the content is the expensive one.
function M.of(pane)
  local revision = neotmux.pane.revision(pane)
  if revision == nil then
    seen[pane] = nil
    return nil
  end
  local it = seen[pane]
  if it == nil or it.revision ~= revision then
    local blocked = asking(pane)
    if blocked == nil then
      seen[pane] = nil
      return nil
    end
    it = { revision = revision, at = os.time(), blocked = blocked }
    seen[pane] = it
  end
  if it.blocked then
    return "blocked"
  end
  if os.time() - it.at < M.working_for then
    return "working"
  end
  return "idle"
end

function M.icon(name)
  return M.icons[name] or M.icons.idle
end

-- The state of a set of panes, which is the state of the one most worth knowing about: a
-- session with anything waiting for you is waiting for you, and one with anything running is
-- running.
function M.worst(panes)
  local worst = "idle"
  for _, pane in ipairs(panes) do
    local one = M.of(pane)
    if one == "blocked" then
      return "blocked"
    elseif one == "working" then
      worst = "working"
    end
  end
  return worst
end

return M

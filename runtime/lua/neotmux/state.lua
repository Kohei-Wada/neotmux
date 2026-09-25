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
--
-- The guess itself now lives in `neotmux.detect`, which reads a screen by rules rather than by
-- pattern, and knows the difference between a spinner's blink and a turn actually finishing.
-- `M.of` asks it first. What follows in this file — the pattern list, `M.working_for` — is what
-- answers for a pane no ruleset in `neotmux.detect.agents` claims: a shell, or an agent nobody
-- has taught neotmux the screen of yet.
--
-- Which panes those are is the ruleset's own answer: `neotmux.detect` asks it, through its
-- optional `claims(info)`, before running a rule. The shipped Claude ruleset names the *shells*
-- and claims every other program, so a pane where you have quit `claude` comes back here — it is
-- the pane you go on working in, and the title Claude left behind would otherwise answer `idle`
-- for it forever — while a pane running the agent under a wrapper of your own is still its.
--
-- Two caches, `neotmux.detect`'s and this file's own, sit behind the one call: each keyed by a
-- pane's revision, each rebuilt only when it moves. They cannot disagree today only because both
-- always recompute the moment the revision they last saw is stale. The crossing from claimed to
-- unclaimed has a test on it now (`state.rs`); the one thing still unenforced is that `claims` is
-- re-asked only when the screen moves, so a pane whose program changed and printed nothing is
-- answered by the last look for as long as that lasts.
local g = neotmux.group("neotmux.state")

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

-- How long after a pane last said something it still counts as working, in milliseconds against
-- `neotmux.now()`. Only the fallback uses it: a pane a ruleset claims is believed outright. A
-- shell that printed a prompt and stopped is idle a moment later; a program printing as it goes
-- stays working between its lines.
M.working_for = 2000

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

-- Whether the pane's last few non-blank lines read as a question. `lines` may already have its
-- blanks stripped (a view handed over by `neotmux.detect`) or not (raw `pane.content`) — either
-- way the last `M.lines` non-empty entries are the ones that matter.
local function asking(lines)
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
  require("neotmux.detect").forget(pane)
end

g.on_event("pane_closed", function(event)
  seen[event.pane] = nil
end)

-- A pane a ruleset in `neotmux.detect.agents` claims is answered there; everything below is
-- what a pane none of them recognise falls back to. `lines`, when given, is the view
-- `neotmux.detect` already read this pane's screen into — reused rather than read again.
local function fallback(pane, lines)
  -- The whole cost control: the lines are built only for a pane that has been fed since it was
  -- last looked at, which on a session nobody is typing into is none of them. The stamp is one
  -- call and no copying; the content is the expensive one.
  local revision = neotmux.pane.revision(pane)
  if revision == nil then
    seen[pane] = nil
    return nil
  end
  local it = seen[pane]
  if it == nil or it.revision ~= revision then
    if lines == nil then
      lines = neotmux.pane.content(pane)
      if lines == nil then
        seen[pane] = nil
        return nil
      end
    end
    it = { revision = revision, at = neotmux.now(), blocked = asking(lines) }
    seen[pane] = it
  end
  if it.blocked then
    return "blocked"
  end
  if neotmux.now() - it.at < M.working_for then
    return "working"
  end
  return "idle"
end

function M.of(pane)
  local detected, view = require("neotmux.detect").of(pane)
  if detected ~= nil then
    return detected
  end
  return fallback(pane, view and view.lines or nil)
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

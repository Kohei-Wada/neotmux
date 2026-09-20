-- Says which pane looks like it is waiting for you.
--
-- Off by default, unlike the other shipped plugins: a status line is a fact about the
-- terminal, while what counts as "waiting for a human" is an opinion about what you run. Turn
-- it on from your own `init.lua`:
--
--   require('neotmux.defaults').watch = true
--
-- Nothing here is a host feature. It is `neotmux.window.panes` and `neotmux.pane.content`
-- read from a tick, a set of patterns, and `neotmux.notify`.
local M = {}

-- Lua patterns, matched against the last `M.lines` lines of a pane. Replace the table to
-- replace the guess:
--
--   require('plugin.watch').patterns = { 'my agent is waiting' }
--
-- These are the prompt shapes that mean a program has stopped and wants an answer. They are a
-- guess about text on a screen and nothing more: a pane showing this repository's own source
-- matches several of them, and an agent that asks its question in words nobody listed here
-- matches none. Narrow the table to what you actually run.
M.patterns = {
  '%[[yYnN]/[yYnN]%]', -- [y/N]
  '%([yYnN]/[yYnN]%)', -- (y/n)
  '%[[yY]es/[nN]o%]',
  '[Dd]o you want to',
  '[Pp]ress %a+ to continue',
  '[Cc]ontinue%?',
  '[Pp]assword:',
  '[Pp]assphrase',
}

-- How many of the pane's last non-blank lines to match against. A prompt is the last thing a
-- program said, and matching the whole screen would fire on anything it ever printed. Blank
-- rows are skipped rather than counted, because a pane that has said one line so far has
-- twenty-three of them below it.
M.lines = 5

-- The pane you are looking at is the one you can already see is waiting.
M.ignore_focused = true

-- The panes worth asking about, and the revision each was last read at. Every pane the host
-- owns is in here, hidden windows included: an agent you started in a window you are not
-- looking at has said nothing yet, and that is exactly the pane worth watching.
local revisions = {}

-- Which panes were waiting last time they were read, so the notification is sent on the
-- change rather than on every frame the prompt is still up.
local waiting = {}

local function matches(line)
  for _, pattern in ipairs(M.patterns) do
    if line:find(pattern) then
      return true
    end
  end
  return false
end

-- Whether the pane is waiting, or nil when it went away between the tick and now.
local function read(pane)
  local lines = neotmux.pane.content(pane)
  if lines == nil then
    return nil
  end
  local looked = 0
  for i = #lines, 1, -1 do
    if lines[i] ~= '' then
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

local function forget(pane)
  revisions[pane] = nil
  waiting[pane] = nil
end

-- Enumeration, not output: a pane is watched from the moment it exists rather than from the
-- moment it first prints something. It changes which panes are read, never how often — the
-- revision gate below still decides that.
local function discover()
  for _, window in ipairs(neotmux.window.list()) do
    local panes = neotmux.window.panes(window)
    if panes then
      for _, pane in ipairs(panes) do
        if revisions[pane] == nil then
          -- Never a real revision, so the pane is read the first time round.
          revisions[pane] = false
        end
      end
    end
  end
end

neotmux.on_event('pane_closed', function(event)
  forget(event.pane)
end)

neotmux.on_tick(function()
  discover()
  local focused = M.ignore_focused and neotmux.pane.current() or nil
  for pane in pairs(revisions) do
    local revision = neotmux.pane.revision(pane)
    if revision == nil then
      forget(pane)
    elseif revision ~= revisions[pane] then
      -- The whole cost control: the lines are built only for a pane that has been fed since
      -- it was last looked at, which on an idle session is none of them.
      revisions[pane] = revision
      local now = read(pane)
      if now == nil then
        forget(pane)
      else
        if now and not waiting[pane] and pane ~= focused then
          neotmux.notify('info', 'pane ' .. pane .. ' looks like it is waiting for you')
        end
        waiting[pane] = now
      end
    end
  end
end)

return M

-- Whether you have seen an agent finish: the `done` state herdr shows, an agent that went idle
-- while nobody was looking at it, until somebody does.
--
--   local seen = require('neotmux.seen')
--   seen.of(pane, state.of(pane))   --> the state, or 'done'
--
-- `of` only reads; what a pane went through is recorded here, and a pane is asked again only when
-- something that can move it happened: its output or its program (`pane_changed`,
-- `pane_info_changed`), its opening, time while it is `working` (a detector's settle and the
-- fallback's grace run out with no output), and somebody coming to look at a pane that is
-- done. A pane is looked at when it is in the current
-- window of a session a client is attached to (`neotmux.session.attached`): every session ticks
-- whether or not anybody is there, so a session nobody is at does not count as being read.
--
-- Only a pane an agent ruleset answers for (`neotmux.detect.agent`) is ever done. A shell goes
-- idle every time a command finishes, and a sidebar lit up by every one of them says nothing.
--
-- Not a detector's answer, which is why it is not in `neotmux.state`: the chime and the watcher
-- ask what a pane is doing, and this is about what you have and have not looked at.
local detect = require("neotmux.detect")
local state = require("neotmux.state")
local g = neotmux.group("neotmux.seen")

local M = {}

-- Moves whenever a pane's state or whether it is done changes, so a reader that keeps what it
-- drew can tell it is stale without asking every pane.
M.generation = 0

-- Per pane, the generation it last moved the count to, so that reader can tell which panes.
M.moved = {}

-- The state each pane was last seen in, the panes that finished unwatched, and which session
-- each pane is in, since a pane is asked in its own session's frame.
local last = {}
local unseen = {}
local owner = {}

-- Per session: its panes, the ones last seen `working` and the ones done, which are the only
-- ones its tick asks again, and whether its first tick has asked every pane yet.
local sessions = {}

local function looked_at(pane)
  local name = neotmux.session.current()
  if name == nil or neotmux.session.attached(name) == 0 then
    return false
  end
  local window = neotmux.window.current()
  for _, shown in ipairs(window and neotmux.window.panes(window) or {}) do
    if shown == pane then
      return true
    end
  end
  return false
end

local function forget(pane)
  local s = sessions[owner[pane]]
  if s then
    s.panes[pane], s.working[pane], s.done[pane] = nil, nil, nil
  end
  owner[pane], last[pane], unseen[pane], M.moved[pane] = nil, nil, nil, nil
  M.generation = M.generation + 1
end

local function session(name)
  local s = sessions[name]
  if s == nil then
    s = { panes = {}, working = {}, done = {} }
    sessions[name] = s
  end
  return s
end

-- What the pane is now, asked in the frame of the session it is in.
local function judge(pane)
  local name = neotmux.session.current()
  if name == nil then
    return
  end
  if neotmux.pane.revision(pane) == nil then
    if owner[pane] ~= nil then
      forget(pane)
    end
    return
  end
  local s = session(name)
  if owner[pane] ~= name then
    local was = sessions[owner[pane]]
    if was then
      was.panes[pane], was.working[pane], was.done[pane] = nil, nil, nil
    end
    owner[pane] = name
    s.panes[pane] = true
  end
  local now = state.of(pane)
  local was = last[pane]
  local done = unseen[pane]
  last[pane] = now
  if now ~= "idle" or looked_at(pane) then
    unseen[pane] = nil
  elseif (was == "working" or was == "blocked") and detect.agent(pane) then
    unseen[pane] = true
  end
  s.working[pane] = now == "working" or nil
  s.done[pane] = unseen[pane]
  if now ~= was or unseen[pane] ~= done then
    M.generation = M.generation + 1
    M.moved[pane] = M.generation
  end
end

for _, name in ipairs({ "pane_changed", "pane_info_changed", "pane_opened" }) do
  g.on_event(name, function(e)
    judge(e.pane)
  end)
end

g.on_event("pane_closed", function(e)
  if owner[e.pane] ~= nil then
    forget(e.pane)
  end
end)

g.on_event("window_closed", function()
  local s = sessions[neotmux.session.current()]
  for pane in pairs(s and s.panes or {}) do
    if neotmux.pane.revision(pane) == nil then
      forget(pane)
    end
  end
end)

g.on_event("session_closed", function(e)
  local s = sessions[e.session]
  for pane in pairs(s and s.panes or {}) do
    forget(pane)
  end
  sessions[e.session] = nil
end)

g.on_event("session_renamed", function(e)
  local s = sessions[e.from]
  sessions[e.session], sessions[e.from] = s, nil
  for pane in pairs(s and s.panes or {}) do
    owner[pane] = e.session
  end
end)

-- What the current session has that time can move, asked again: run by this module's tick in
-- every session every frame, whether or not anything is drawing it, so the palette on `C-b g`
-- knows with the sidebar off and a reader that last looked long ago is not mistaken for a
-- transition. Ticks run in the order their files began loading, so a reader that ticks before
-- this module and must draw this frame's answer calls it first; a second call finds nothing new.
function M.update()
  local name = neotmux.session.current()
  if name == nil then
    return
  end
  local s = session(name)
  if not s.swept then
    s.swept = true
    for _, pane in ipairs(require("neotmux.panes").all()) do
      judge(pane)
    end
    return
  end
  for pane in pairs(s.working) do
    judge(pane)
  end
  -- However a done pane came to be looked at: a window or focus moving, a pane moved into
  -- view, a client attaching.
  if next(s.done) and neotmux.session.attached(name) > 0 then
    local window = neotmux.window.current()
    for _, pane in ipairs(window and neotmux.window.panes(window) or {}) do
      if unseen[pane] then
        judge(pane)
      end
    end
  end
end

g.on_tick(M.update)

-- `state`, or `'done'` for an idle pane that finished while nobody was looking. A read: it
-- changes nothing, however often and from wherever it is asked.
function M.of(pane, now)
  if now == "idle" and unseen[pane] then
    return "done"
  end
  return now
end

return M

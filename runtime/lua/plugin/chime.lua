-- A sound when a pane starts waiting for you.
--
-- The point of the detector: a pane you are not looking at stops being a pane you have to keep
-- looking at. `plugin.watch` puts the same change on the screen; this one puts it in the room,
-- which is the only half that works while your eyes are somewhere else.
--
-- One sound per *entry* into a state. A pane that is still blocked on the next frame is the
-- same wait, not a second one, so nothing sounds again until it has been something else in
-- between. The state itself is `neotmux.state`'s, recomputed whenever the pane's screen moves
-- — nothing here forgets a pane to force a fresh look, because forgetting drops the hold that
-- keeps a spinner's blink from reading as a finished turn.
--
--   require('plugin.chime').only_when_away = false   -- chime for the pane you are in, too
--   require('plugin.chime').sounds.idle = nil        -- only the waiting, not the finishing
--
-- Every pane the session owns is watched, hidden windows included — the same reach
-- `plugin.watch` has, and for the same reason: another window is exactly where you are when you
-- have stopped looking at the one an agent is in.
--
-- Nothing here is a host feature: `neotmux.window.list`, `neotmux.job.start` and
-- `neotmux.notify`, over the shared guess.
local detect = require("neotmux.detect")
local state = require("neotmux.state")

local M = {}

-- The layer this file was loaded from, so that a sound is found beside the plugin that ships
-- it rather than under `neotmux.home` — a user's own layer shadowing this file gets its own
-- sounds with it. `require`'s loader is the only thing that knows which layer won, and
-- `package.searchpath` is how to ask it.
M.dir = (package.searchpath("plugin.chime", package.path) or ""):match("^(.*)/lua/plugin/chime%.lua$")

-- Relative to `M.dir`; an absolute path is taken as it stands, which is how you point at a
-- sound of your own. A state with no entry here makes no sound — `working` is every state
-- there is, most of the time, and a sound for it would be a sound for nothing.
M.sounds = { blocked = "sounds/request.mp3", idle = "sounds/done.mp3" }

-- Tried in order, and the first that plays a file to the end is the one used from then on.
-- Each entry is a program and the arguments that make it quiet; the path is appended.
M.players = {
  { "paplay" },
  { "pw-play" },
  { "ffplay", "-nodisp", "-autoexit", "-loglevel", "quiet" },
  { "mpg123", "-q" },
  { "mpv", "--no-video", "--really-quiet" },
}

-- The pane you are looking at is the one you can already see has stopped.
M.only_when_away = true

-- What each pane was last seen doing, so a sound is made on the change rather than on every
-- frame the prompt is still up.
local seen = {}

-- The player being used, as an index into `M.players`, and whether the walk has run out. A
-- machine with no player at all is a fact about the machine: said once, then dropped.
local using = 1
local hopeless = false

neotmux.on_event("pane_closed", function(event)
  seen[event.pane] = nil
  -- The hold is worth keeping for a live pane and worth nothing for a dead one: an id is
  -- reused, and what the pane before it was doing is not evidence about its successor.
  state.forget(event.pane)
end)

local function file(name)
  if name:sub(1, 1) == "/" or M.dir == nil then
    return name
  end
  return M.dir .. "/" .. name
end

-- A player is judged by how it *ends*, not by whether it started: one that is not installed
-- fails at `start`, and one that is installed but cannot decode an mp3 starts happily and
-- exits non-zero a moment later. Both mean the same thing — try the next one.
--
-- What must not move the walk on is an ending that says nothing about the player. A job killed
-- by a session ending, or by the output limit, answers `on_exit` with a nil code; a `start`
-- refused because this source already has `job_concurrency` jobs running, or because the
-- session is going away, answers with no handle either. Reading any of those as "this program
-- does not work" writes off a player that works perfectly well, for the life of the server —
-- so only a *number* that is not zero, and only a refusal that names the spawn, count as
-- evidence. Everything else leaves the walk where it is, to be tried again next time.
local function player_failed(why)
  -- `neotmux.job.start`'s own wording for a spawn that failed. The two refusals it makes
  -- before ever reaching the program — "the session is ending", "too many jobs: N already
  -- running" — are facts about this layer, and matching the spawn positively is what keeps a
  -- refusal added later from being read as a verdict on a program nobody tried to run.
  return type(why) == "string" and why:find("^could not start ") ~= nil
end

local function play(path, from)
  local spec = M.players[from]
  if spec == nil then
    -- Two walks can run out in the same frame, and one machine with no player is one thing to
    -- be told about.
    if hopeless then
      return
    end
    hopeless = true
    neotmux.notify("error", "plugin.chime: no player could play " .. path .. "; set .players")
    return
  end
  local cmd = { table.unpack(spec) }
  cmd[#cmd + 1] = path
  local job, why = neotmux.job.start(cmd, {
    on_exit = function(code)
      if code == 0 then
        using = from
      elseif type(code) == "number" then
        play(path, from + 1)
      end
    end,
  })
  if job == nil and player_failed(why) then
    play(path, from + 1)
  end
end

local function watch(pane, focused)
  local now = state.of(pane)
  -- A shell reaches `idle` at the end of every command it runs — that is what
  -- `neotmux.state`'s fallback is for — and a sound on that edge would chime for an `ls` in a
  -- pane nobody is looking at. An `idle` worth hearing is one the *detector* answered: a turn
  -- that finished, on a screen a ruleset in `neotmux.detect.agents` read. A question left
  -- standing is different, and still sounds for anything, because a shell asking one is
  -- waiting for you exactly as an agent is.
  if now == "idle" and detect.of(pane) == nil then
    now = nil
  end
  if now ~= seen[pane] and not hopeless then
    local sound = M.sounds[now]
    if sound ~= nil and pane ~= focused then
      play(file(sound), using)
    end
  end
  seen[pane] = now
end

neotmux.on_tick(function()
  local focused = M.only_when_away and neotmux.pane.current() or nil
  for _, window in ipairs(neotmux.window.list()) do
    for _, pane in ipairs(neotmux.window.panes(window) or {}) do
      watch(pane, focused)
    end
  end
end)

return M

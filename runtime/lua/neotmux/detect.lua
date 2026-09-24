-- What a pane looks like it is doing, read off the screen by rules.
--
-- `neotmux.state` guesses: eight patterns over the last five lines, enough to tint a row in a
-- sidebar and not enough to wake you up. This is the machine meant to replace that guess. It
-- holds no knowledge of any program — a *view* of a pane, a rule engine over it, and a hold
-- that keeps a spinner's blink from reading as "done". What a particular agent's screen looks
-- like lives in `neotmux/agents/*.lua`, one file per program, so that teaching neotmux a new
-- agent is a file somebody adds and not a patch to this one.
--
--   local detect = require('neotmux.detect')
--   detect.of(pane)  --> 'working' | 'blocked' | 'idle', or nil when nothing answered
--
-- A rule is a table:
--
--   { id = 'spinner', state = 'working', priority = 9, visible = 'working',
--     match = function(view) return ... end }
--
-- `priority` decides, not where the rule sits in the table. `state` is the answer, and a rule
-- with none says "this screen is not a state" and stops the search. `visible` is the state the
-- screen is *showing*, spelled out: it is read, not merely tested, and only `'idle'` or
-- `'blocked'` change anything — they release the hold below at once, because a prompt on the
-- screen is evidence rather than the absence of it. `visible = 'working'` on a rule that
-- answers `idle` is a rule with a typo in it, and gets held like any other unevidenced fall.
--
-- A view is read-only to a rule. Every region is cut from one list, that list is handed out as
-- `view.lines`, and the whole view is kept until the screen moves — so a rule that does
-- `table.remove(view.lines)` to ignore the last line corrupts every region for every rule
-- below it, and keeps corrupting them until the pane is fed again. Copy before you cut.
--
-- The shape is herdr's (herdrdev/herdr, Apache-2.0): a rule with a priority, a state and the
-- screen regions it reads is `src/detect/manifests/*.toml`, and the hold below — a fall to
-- idle disbelieved until it settles, unless the screen says idle in so many words — is
-- `src/pane/agent_detection.rs`.
local M = {}

-- How long a pane that was working keeps counting as working after the screen stops saying
-- so, in milliseconds. Between two frames of a spinner, or two lines of an agent's output,
-- there is a screen with nothing on it that means "busy"; a notification fired on that blink
-- is why this kind of detector gets turned off.
M.settle = 700

-- Rulesets by name. One entry per program neotmux knows the screen of; teaching it another is
-- a file in `neotmux/agents/` and a line here.
M.agents = { claude = require("neotmux.agents.claude") }

local SPACE = { [0x20] = true, [0x09] = true }
local DASH = { [0x2d] = true, [0x2500] = true } -- - ─
local TOP, BOTTOM = 0x256d, 0x2570 -- ╭ ╰

-- Codepoints, not a character class: `─` is three bytes, and `[─]` is a class of those three
-- bytes on their own, which matches half of every other box-drawing character on the screen.
local function ink(line)
  if utf8.len(line) == nil then
    return nil -- a pane holding bytes that are not text has no regions worth reading
  end
  local codes = {}
  for _, code in utf8.codes(line) do
    codes[#codes + 1] = code
  end
  return codes
end

-- A line whose only ink is three or more dashes: what a program draws to say that everything
-- above it is over.
local function is_rule(line)
  local codes = ink(line)
  if codes == nil then
    return false
  end
  local dashes = 0
  for _, code in ipairs(codes) do
    if DASH[code] then
      dashes = dashes + 1
    elseif not SPACE[code] then
      return false
    end
  end
  return dashes >= 3
end

local function opens_with(line, want)
  local codes = ink(line)
  if codes == nil then
    return false
  end
  for _, code in ipairs(codes) do
    if not SPACE[code] then
      return code == want
    end
  end
  return false
end

-- A boxed line without its `│` edges, which are the box and not what a rule is reading.
local function unedge(line)
  local inner = line:gsub("^%s*\u{2502}", ""):gsub("\u{2502}%s*$", "")
  return (inner:match("^%s*(.-)%s*$"))
end

-- What a rule reads. Built once per look and handed to every rule, with each region computed
-- on first ask: a rule that never looks at the prompt box costs nothing for it.
function M.view(pane)
  local lines = neotmux.pane.content(pane)
  if lines == nil then
    return nil
  end
  local full = {}
  for _, line in ipairs(lines) do
    if line:match("%S") then
      full[#full + 1] = line
    end
  end

  -- Blank lines are dropped before any region is cut, so that "the last two lines" means the
  -- last two a program wrote rather than however much empty screen is below them.
  local view = { lines = full, title = neotmux.pane.title(pane) or "" }

  function view.tail(n)
    local out = {}
    for i = math.max(1, #full - n + 1), #full do
      out[#out + 1] = full[i]
    end
    return out
  end

  local after
  function view.after_rule()
    if after == nil then
      after = {}
      local from = 1
      for i = #full, 1, -1 do
        if is_rule(full[i]) then
          from = i + 1
          break
        end
      end
      for i = from, #full do
        after[#after + 1] = full[i]
      end
    end
    return after
  end

  -- The last box on the screen, without its edges. An unclosed box — one whose bottom has
  -- scrolled off, which is what a box being typed into looks like — is still a box, so the
  -- region runs to the end of the screen rather than coming back empty. The cost of that is a
  -- stale `╭` high up dragging everything below it in, so a rule that cares should check what
  -- it got rather than that it got something.
  local box
  function view.prompt_box()
    if box == nil then
      box = {}
      local top
      for i = #full, 1, -1 do
        if opens_with(full[i], TOP) then
          top = i
          break
        end
      end
      for i = (top or #full) + 1, #full do
        if opens_with(full[i], BOTTOM) then
          break
        end
        box[#box + 1] = unedge(full[i])
      end
    end
    return box
  end

  return view
end

-- A ruleset does not change between looks, so it is sorted once and kept against the table
-- itself — weakly, so that a ruleset nobody holds any more does not live here forever.
local ordered = setmetatable({}, { __mode = "k" })

local function by_priority(rules)
  local sorted = ordered[rules]
  if sorted == nil then
    sorted = {}
    for index, rule in ipairs(rules) do
      sorted[index] = { rule = rule, index = index }
    end
    -- `table.sort` is not stable, so the tie is broken by where the rule was written: two
    -- rules of equal priority must not swap places between looks.
    table.sort(sorted, function(a, b)
      local left, right = a.rule.priority or 0, b.rule.priority or 0
      if left ~= right then
        return left > right
      end
      return a.index < b.index
    end)
    ordered[rules] = sorted
  end
  return sorted
end

-- The rules that have raised, so that each one is reported once and not once a frame. Keyed by
-- the rule table rather than by `rule.id`: ids collide across rulesets, and the rule most likely
-- to raise is the malformed one that has no id at all. Weak, so a ruleset that goes away does.
local complained = setmetatable({}, { __mode = "k" })

-- The first rule that matches, by priority. It answers `state, id, visible` — and a rule with
-- no state answers `nil, id`, which says "this screen is not a state": a transcript scrolled
-- back, a menu over the top. That stops the search rather than falling through to a lower
-- rule, because a screen the ruleset has recognised is not one to keep guessing about.
function M.judge(rules, view)
  if view == nil then
    return nil
  end
  for _, entry in ipairs(by_priority(rules)) do
    local rule = entry.rule
    local ok, hit = pcall(rule.match, view)
    if not ok then
      -- One bad rule must not take the detector down with it, and must say which one it was —
      -- once. This runs for every pane on every frame, and a rule that raises on one screen
      -- raises on the next thousand.
      if not complained[rule] then
        complained[rule] = true
        local which = "neotmux.detect: rule '" .. tostring(rule.id) .. "'"
        neotmux.notify("error", which .. ": " .. tostring(hit))
      end
    elseif hit then
      return rule.state, rule.id, rule.visible
    end
  end
  return nil
end

-- The last look at a pane, keyed by the revision it was taken at, and the state the pane is
-- being held at. Two tables because they expire differently: the look is stale the moment the
-- screen moves, the hold outlives any number of screens.
local seen = {}
local held = {}

-- Both of them: this is the call for "start over on this pane", and a forget that dropped the
-- look but kept the hold would answer the next question out of a screen it had not read.
function M.forget(pane)
  seen[pane] = nil
  held[pane] = nil
end

-- `pairs` does not walk a table the same way twice, and two rulesets that both answer would
-- then answer differently between looks.
local function names()
  local out = {}
  for name in pairs(M.agents) do
    out[#out + 1] = name
  end
  table.sort(out)
  return out
end

local function hold(pane, state, visible)
  local was = held[pane]
  if state == nil then
    return was and was.state or nil
  end
  local now = neotmux.now()
  -- Only the fall out of working is disbelieved, and only when the screen does not say in so
  -- many words what it is doing instead. `visible` is a state name, not a flag: a rule that
  -- claims to show `working` while answering `idle` is a rule with a typo in it, and holding
  -- it is the better of the two wrong answers.
  local shows = visible == "idle" or visible == "blocked"
  if was ~= nil and was.state == "working" and state == "idle" and not shows then
    if now - was.since < M.settle then
      return was.state
    end
  end
  -- Mutated rather than replaced: `of` runs this on every look at every pane, and a fresh
  -- table per look is a few hundred a second on a session nobody is even typing into.
  if was == nil then
    held[pane] = { state = state, since = now }
  else
    was.state, was.since = state, now
  end
  return state
end

-- The same cost control as `neotmux.state`: the screen is read only for a pane that has been
-- fed since it was last looked at. The hold runs on every call regardless, because a pane
-- that has stopped changing is exactly the one whose settle has to expire.
function M.of(pane)
  local revision = neotmux.pane.revision(pane)
  if revision == nil then
    seen[pane] = nil
    held[pane] = nil
    return nil
  end
  local it = seen[pane]
  if it == nil or it.revision ~= revision then
    it = { revision = revision }
    local view = M.view(pane)
    if view ~= nil then
      for _, name in ipairs(names()) do
        local state, id, visible = M.judge(M.agents[name], view)
        if id ~= nil then
          it.state, it.visible = state, visible
          break
        end
      end
    end
    seen[pane] = it
  end
  return hold(pane, it.state, it.visible)
end

return M

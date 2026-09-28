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
-- `of` answers a second value too: the view it judged the pane from, or the one it last judged
-- it from if this look was a cache hit, or nil if the pane has none (gone, or never read).
-- `neotmux.state` takes it to avoid reading a pane's content twice on the common path — a pane
-- no ruleset claims, which is the ordinary shell.
--
-- `detect.agent(pane)` names the ruleset that answered at that look — `'claude'` — or nil, which
-- is how a list of agents tells an agent from a pane the fallback is guessing about.
--
-- A rule is a table:
--
--   { id = 'spinner', state = 'working', priority = 9, visible = 'working',
--     match = function(view) return ... end }
--
-- A ruleset may carry one field of its own beside its rules:
--
--   claims = function(info) return not shells[info.command] end
--
-- asked with `neotmux.pane.info(pane)` before any rule runs. A ruleset that answers false is
-- skipped whole and the pane falls through to whatever `neotmux.state` does with a pane nobody
-- claims. Without it a ruleset speaks for every pane on the screen alone, which is how a pane
-- that once ran an agent goes on being answered for out of what the agent left behind — a
-- title nothing resets when the program exits.
--
-- Asked once per *revision*, not once per call: it sits inside `of`'s cache, so a pane whose
-- program changed and printed nothing keeps the last look's verdict until the next byte. And
-- `pane.info` is itself the host's, refreshed on a 250 ms cadence, so even a fresh revision can
-- be judged against a command up to that old. Both windows are a fraction of a second wide in
-- the one case this exists for — quitting an agent redraws a prompt, and typing echoes.
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
-- `view.title` is the one region that is not read off the screen, and the one that can be a
-- lie: a terminal keeps the last title it was given, and the program that gave it is long
-- gone. `of` blanks it for a pane whose command has changed since — see `titled` below — so a
-- rule may read it as what is running now rather than as what ran once.
--
-- The shape is herdr's (herdrdev/herdr, Apache-2.0): a rule with a priority, a state and the
-- screen regions it reads is `src/detect/manifests/*.toml`, and the hold below — a fall to
-- idle disbelieved until it settles, unless the screen says idle in so many words — is
-- `src/pane/agent_detection.rs`.
local g = neotmux.group("neotmux.detect")

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
local RULE = 0x2500 -- ─

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

-- A line whose ink starts with `─`: what a program draws to say that what is above it is over
-- and what is below it is something else. Only that one character counts — `╭───╮` is a box and
-- `---` is a sentence with a dash in it, and a screen is full of both.
local function is_rule(line)
  return opens_with(line, RULE)
end

-- A line without the `│` edges a program might have drawn around it, trimmed. Nothing Claude
-- Code draws has them and no test exercises the stripping: it is a courtesy to the next program
-- somebody points this at, and the trim is the part that earns its keep.
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

  -- What is written between the second rule from the bottom and the one below it. A program
  -- that draws a prompt draws it between two rules and draws whatever it wants an answer about
  -- under a single one — so a screen with fewer than two rules has no prompt on it at all, and
  -- the region is empty rather than a guess. That one fact is what keeps a dialog waiting on
  -- you from being read as an empty prompt nobody has typed into.
  local box
  function view.prompt_box()
    if box == nil then
      box = {}
      local last, second
      for i = #full, 1, -1 do
        if is_rule(full[i]) then
          if last == nil then
            last = i
          else
            second = i
            break
          end
        end
      end
      if second ~= nil then
        for i = second + 1, last - 1 do
          box[#box + 1] = unedge(full[i])
        end
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

-- Whether a ruleset will speak for this pane at all, asked before any of its rules run. A
-- ruleset with no `claims` speaks for every pane, which is what one that is only about what is
-- drawn wants. The shipped one declines the shells — a pane whose agent has quit is back at a
-- prompt reporting `bash`, and is then answered by the fallback rather than by the agent's
-- leavings — and claims everything else, because a wrapper is what the pane reports when an
-- agent is started through one.
--
-- A `claims` that raises declines, and says which ruleset it was — once, for the same reason a
-- rule that raises is reported once. The ruleset is the thing that is broken, and skipping it
-- leaves the pane where it would have been had nobody written it.
function M.claimed(name, rules, pane)
  local ask = rules.claims
  if ask == nil then
    return true
  end
  -- The `info` read is inside the `pcall` too: it is `neotmux.pane.info`, which answers nil
  -- rather than raising today, and a change that made it raise must not take a look down.
  local ok, yes = pcall(function()
    return ask(neotmux.pane.info(pane))
  end)
  if not ok then
    if not complained[rules] then
      complained[rules] = true
      local which = "neotmux.detect: ruleset '" .. tostring(name) .. "' claims"
      neotmux.notify("error", which .. ": " .. tostring(yes))
    end
    return false
  end
  return yes and true or false
end

-- The last look at a pane, keyed by the revision it was taken at, and the state the pane is
-- being held at. Two tables because they expire differently: the look is stale the moment the
-- screen moves, the hold outlives any number of screens.
local seen = {}
local held = {}

-- And the title each pane was last seen wearing, against the program that was running when it
-- was. A terminal keeps the last title it was given forever, so a title is the only thing on a
-- pane that can outlive the program that wrote it — which is why a title rule, alone among
-- rules, can answer out of a screen nobody is looking at any more.
local titled = {}

-- All three: this is the call for "start over on this pane", and a forget that dropped the
-- look but kept the hold would answer the next question out of a screen it had not read.
function M.forget(pane)
  seen[pane] = nil
  held[pane] = nil
  titled[pane] = nil
end

-- This module's own, not its consumers': an id is reused, and a dead pane's hold is no evidence
-- about the next pane given it, whichever plugins happen to be loaded.
g.on_event("pane_closed", function(event)
  M.forget(event.pane)
end)

-- What the pane is running, or nil for a pane nothing can be read about. `pane.info` answers
-- nil rather than raising today; the pcall is for the day it does not.
local function command_of(pane)
  local ok, info = pcall(neotmux.pane.info, pane)
  if not ok or info == nil then
    return nil
  end
  return info.command
end

-- Whether the title on this pane is still the title of the program that is running on it.
--
-- A program sets the title and then exits; the next program inherits it and mostly never sets
-- one of its own. So a pane that ran Claude Code and now runs `less` wears `✳ …`, and
-- `osc_title_idle` would answer `idle` for it until the end of the session — the pane reads
-- done when it is a pager, and a question underneath it goes unheard. That is the same fault
-- `claims` was built to fix, back for every program that is not a shell.
--
-- The test is the title against the program it was first seen under: the same title under a
-- different command is a title nobody has refreshed, and is treated as no title at all until
-- the program writes one. The stored command is deliberately not updated while the title is
-- stale, so it stays stale rather than being adopted by whatever is running one look later.
--
-- It cannot see a program that sets the very same title its predecessor did, and it is judged
-- against a `pane.info` up to the host's 250 ms old — the same window `claims` is judged in. `pane.info` is `/proc`, so on anything but Linux the command is always nil
-- and this seam, like `claims` above it, is inert.
local function title_is_the_running_program(pane, title)
  local command = command_of(pane)
  local was = titled[pane]
  if was == nil or was.title ~= title then
    titled[pane] = { title = title, command = command }
    return true
  end
  -- A command nobody could read is not an owner, and is not written down as one.
  --
  -- The reason is the platform, not the guess. `pane.info` is `/proc`: off Linux it answers nil
  -- for every pane, always. A rule that distrusted a title it could not match to a program
  -- would therefore blank every title on every non-Linux host and take both title rules with
  -- it — the detector would lose its two best-evidenced rules everywhere but here, to close a
  -- hole that is only reachable on the platform where the command *can* be read.
  --
  -- This is the looser of the two, measurably. Writing the next readable command down instead
  -- would let ownership come off again one program later, and a `Continue? [y/N]` after that
  -- would be heard; trusting keeps the title through all of them. `state.rs` has the four looks
  -- that tell the two apart, and pins this one with the question it misses in plain sight.
  if command == nil or was.command == nil then
    return true
  end
  return command == was.command
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

-- `matched` is whether a rule fired at all, which is a different fact from whether it named a
-- state. A rule that fired and declined to say — a transcript scrolled back, a menu over the
-- top — leaves the previous answer standing, because the pane is still the ruleset's and the
-- screen simply is not evidence. Nothing firing is not that: the ruleset has nothing to say
-- about this screen, and an answer it gave about some earlier one must not outlive it, or a
-- pane that has ever been claimed is never unclaimed again.
local function hold(pane, state, visible, matched)
  local was = held[pane]
  if not matched then
    held[pane] = nil
    return nil
  end
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
-- fed since it was last looked at, or whose program has changed since — an agent quit back to
-- a shell that has drawn nothing yet is a different pane to `claims`, on the same screen. The
-- hold runs on every call regardless, because a pane that has stopped changing is exactly the
-- one whose settle has to expire. The view built here is kept on `it` rather than thrown away,
-- so a cache hit still has one to hand back.
function M.of(pane)
  local revision = neotmux.pane.revision(pane)
  if revision == nil then
    M.forget(pane)
    return nil
  end
  local command = command_of(pane)
  local it = seen[pane]
  if it == nil or it.revision ~= revision or it.command ~= command then
    it = { revision = revision, command = command, view = M.view(pane) }
    if it.view ~= nil then
      if not title_is_the_running_program(pane, it.view.title) then
        it.view.title = ""
      end
      for _, name in ipairs(names()) do
        local rules = M.agents[name]
        if M.claimed(name, rules, pane) then
          local state, id, visible = M.judge(rules, it.view)
          if id ~= nil then
            it.state, it.visible, it.matched, it.agent = state, visible, true, name
            break
          end
        end
      end
    end
    seen[pane] = it
  end
  return hold(pane, it.state, it.visible, it.matched), it.view
end

-- Which ruleset answered for the pane at the last look: its name in `M.agents`, or nil where
-- none did. Read off the cache `of` fills, so it costs nothing after `of` — and knows nothing of
-- a pane `of` has not been asked about.
function M.agent(pane)
  local it = seen[pane]
  return it and it.agent or nil
end

return M

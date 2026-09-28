-- What a Claude Code screen means, as eleven rules.
--
-- Ported from herdr (herdrdev/herdr, Apache-2.0), `src/detect/manifests/claude.toml`. The ids
-- and the priorities are the manifest's on purpose: they are what lets a reader hold this file
-- against the TOML it came from and see whether the two still say the same thing.
--
-- Where they no longer do, and deliberately: `legacy_no_prompt_blocker` requires the screen to
-- have drawn a `─`, which the manifest does not ask for, and reads an option line rather than
-- the word "yes" anywhere. herdr's rules run against a pane it knows is an agent's; these run
-- against every pane that is not a shell, and the manifest's backstop, taken literally, reads
-- a page of prose about permission prompts as a permission prompt.
--
-- Two of the manifest's rules are not ported. `background_agents_working` and
-- `background_mcp_task_working` read the shapes Claude Code draws for work it is running out of
-- sight, and nobody here has a screen of either to check a rule against — a rule written from a
-- pattern alone is one that only ever matches the fixture written from the same pattern.
--
-- Four of the fixtures in `neotmux-lua/tests/detect.rs` are screens captured off a real
-- Claude Code (v2.1.281) and kept in this plan's `captures/`; the rest are written from the
-- manifest's patterns, and say what these rules mean rather than what Claude draws.
--
--   detect.agents = { claude = require('neotmux.agents.claude') }

-- Codepoints, not a pattern class: `✳` is three bytes, and `[✳✶✻]` is a class of those bytes
-- taken singly, which matches a byte of nearly every other symbol on the screen.
--
-- Split in two because the ruleset is now asked about every pane that is not a shell, and half
-- of these characters belong to everyone. `✳ ✶ ✻ ✽ ✢` are typographic asterisks nobody types by
-- accident; `*` and `·` are a markdown bullet, a `git log --graph` edge and a separator in
-- half the prose on disk. A frame is a frame either way — `* Actioning… (3s · ↓ 16 tokens)` is
-- captured off a real Claude Code and lives in `tests/support/mod.rs` — so the ambiguous two
-- are kept and made to show the timer Claude draws with them instead.
local SPINNER = {
  [0x2722] = true, -- ✢
  [0x2733] = true, -- ✳
  [0x2736] = true, -- ✶
  [0x273b] = true, -- ✻
  [0x273d] = true, -- ✽
}
local AMBIGUOUS = {
  [0x2a] = true, -- *
  [0xb7] = true, -- ·
}

-- The elapsed-time counter Claude Code puts after the verb: `(3s · ↓ 16 tokens)`. A bullet in a
-- document has a sentence there, not a clock.
local TIMER = "%(%d+s"
local PAUSE, PLAY = 0x23f8, 0x23f5 -- ⏸ ⏵
local RULE = 0x2500 -- ─
local CHEVRON = 0x276f -- ❯
local STAR = 0x2733 -- ✳
local ELLIPSIS = "\u{2026}" -- …

-- The hints a menu draws to say that the answer is picked rather than typed.
local NAVIGATION = { "\u{2191}/\u{2193}", "\u{2191}\u{2193}", "arrow keys", "to navigate" }

-- A copy: every region is cut from the view's one list, and lowering it in place would hand the
-- rules below this one a screen nobody wrote.
local function lower(lines)
  local out = {}
  for index, line in ipairs(lines) do
    out[index] = line:lower()
  end
  return out
end

local function holds(lines, needle)
  for _, line in ipairs(lines) do
    if line:find(needle, 1, true) then
      return true
    end
  end
  return false
end

local function holds_any(lines, needles)
  for _, needle in ipairs(needles) do
    if holds(lines, needle) then
      return true
    end
  end
  return false
end

local function any_line(lines, matches)
  for _, line in ipairs(lines) do
    if matches(line) then
      return true
    end
  end
  return false
end

-- The first codepoint of a line that is not a space, or nil for a line with no ink and for one
-- holding bytes that are not text.
local function first_ink(line)
  if utf8.len(line) == nil then
    return nil
  end
  for _, code in utf8.codes(line) do
    if code ~= 0x20 and code ~= 0x09 then
      return code
    end
  end
  return nil
end

-- A title whose first character `wanted` accepts, followed by a space. The title is one string
-- rather than a screen, so the character is read off it directly.
local function title_opens(title, wanted)
  if title == "" or utf8.len(title) == nil then
    return false
  end
  if not wanted(utf8.codepoint(title, 1)) then
    return false
  end
  local after = utf8.offset(title, 2)
  return after ~= nil and title:sub(after, after) == " "
end

-- `❯ 1. Yes`, with the chevron cut off first: `❯` is three bytes, so `❯?` would make only the
-- last of them optional. The `│` that goes with it is defensive — no screen anybody has
-- captured puts an option inside a box — and costs nothing to keep. `numbered` is for the rule
-- that wants the option list rather than any line that happens to begin with a yes.
--
-- `%f[%W]` is the whole of the word: without it `no` matched "note that…" and "nothing here",
-- which is most of a page of prose about this very file. With it, this is still only the
-- *shape* of an answer — "No such file or directory" is a sentence and passes — so it is
-- corroboration for a rule, never evidence on its own.
local function is_option(line, numbered)
  local body = line:gsub("^%s*\u{2502}", ""):gsub("^%s*\u{276f}", "")
  local digit = numbered and "%d" or "%d?"
  return body:match("^%s*" .. digit .. "%.?%s*yes%f[%W]") ~= nil
    or body:match("^%s*" .. digit .. "%.?%s*no%f[%W]") ~= nil
end

-- A line that is only a chevron: the empty input box, and the one screen the legacy blocker
-- below must not fire on. The `│` edges are the box rather than what the line says.
local function is_bare_prompt(line)
  return line:match("^%s*\u{2502}?%s*\u{276f}%s*\u{2502}?%s*$") ~= nil
end

local claude = {
  -- The title outranks everything on the screen: Claude Code spins it while a turn runs, and it
  -- keeps spinning over a transcript, a menu, or whatever else the screen has been given to.
  {
    id = "osc_title_working",
    state = "working",
    priority = 1100,
    visible = "working",
    match = function(view)
      return title_opens(view.title, function(code)
        return (code >= 0x2800 and code <= 0x28ff) -- braille, the spinner
          or (code >= 0x25d0 and code <= 0x25d3) -- ◐◑◒◓, the older one
      end)
    end,
  },

  -- Scrolled back into the transcript: whatever the pane was doing, this screen is not it.
  {
    id = "transcript_viewer",
    priority = 1000,
    match = function(view)
      local tail = lower(view.tail(3))
      return holds(tail, "showing detailed transcript")
        and holds_any(tail, { "ctrl+o", "ctrl+e", "\u{2191}\u{2193} scroll", "? for shortcuts" })
    end,
  },

  -- A form waiting on an answer. "Esc to cancel" alone is on half the screens Claude draws, so
  -- it counts only next to the hint that says something is there to confirm or to pick.
  {
    id = "live_blocked_form",
    state = "blocked",
    priority = 980,
    visible = "blocked",
    match = function(view)
      local after = lower(view.after_rule())
      if not holds(after, "esc to cancel") then
        return false
      end
      if holds(after, "enter to confirm") then
        return true
      end
      return holds(after, "enter to select") and holds_any(after, NAVIGATION)
    end,
  },

  -- The `/btw` overlay: a note typed at a turn that is still running.
  --
  -- The manifest's other half — a line ending "esc to close" — is deliberately not ported. This
  -- rule sits above the prompt and above both permission rules, and "esc to close" is a hint any
  -- overlay might carry: were a permission screen ever to show it, a pane waiting for you would
  -- read as working and nothing would make a sound, which is this whole feature failing in the
  -- one direction that matters. Nobody here has a capture of `/btw` at all, so the half that is
  -- kept is the half that cannot be mistaken for something else.
  {
    id = "btw_overlay_working",
    state = "working",
    priority = 975,
    visible = "working",
    match = function(view)
      return any_line(lower(view.tail(5)), function(line)
        return line:match("^%s*/btw") ~= nil
      end)
    end,
  },

  -- A turn in progress: the interrupt hint against a paused or running marker, or a spinner
  -- line with its trailing ellipsis. The ellipsis is the whole of it — Claude Code leaves the
  -- *finished* turn on the screen under the same glyph (`✻ Cogitated for 6s · done 9:01 PM`),
  -- so a rule that fired on the glyph alone would read every idle screen as working.
  --
  -- This one answers `working` at 970, above both permission rules, so a screen it reads wrong
  -- is a pane waiting for you that nothing says anything about. That is what the timer on the
  -- ambiguous frames is for: `* Installing dependencies…` in a pager is a bullet, and a bullet
  -- must not be able to hide a question.
  {
    id = "live_turn_working",
    state = "working",
    priority = 970,
    visible = "working",
    match = function(view)
      return any_line(lower(view.tail(12)), function(line)
        local ink = first_ink(line)
        if ink == nil then
          return false
        end
        if (ink == PAUSE or ink == PLAY) and line:find("esc to interrupt", 1, true) then
          return true
        end
        if line:find(ELLIPSIS, 1, true) == nil then
          return false
        end
        if SPINNER[ink] then
          return true
        end
        return AMBIGUOUS[ink] ~= nil and line:find(TIMER) ~= nil
      end)
    end,
  },

  -- The prompt with nothing running behind it: a `❯` between the two rules Claude Code draws
  -- around what you type. A dialog is drawn under a *single* rule and so has no prompt box at
  -- all, which is what keeps one from reading as an empty prompt; the exclusions below are for
  -- the menus that are drawn in the prompt's own place.
  {
    id = "live_prompt_box",
    state = "idle",
    priority = 950,
    visible = "idle",
    match = function(view)
      local box = lower(view.prompt_box())
      if not any_line(box, function(line)
        return first_ink(line) == CHEVRON
      end) then
        return false
      end
      return not holds_any(box, {
        "enter to select",
        "esc to cancel",
        "arrow keys",
        "\u{2191}/\u{2193} to navigate",
      })
    end,
  },

  -- Picking a model is a menu over the top, not a state: the turn underneath it is whatever it
  -- was. Told apart from a permission prompt, which is drawn the same way and is a state. Read
  -- below the last rule only: this rule answers "not a state", so its words left in scrollback
  -- would otherwise hush a dialog drawn under them.
  {
    id = "model_picker_menu",
    priority = 900,
    match = function(view)
      local lines = lower(view.after_rule())
      return holds(lines, "select model")
        and holds(lines, "enter to set as default")
        and holds(lines, "esc to cancel")
        and not holds(lines, "do you want to proceed?")
    end,
  },

  -- A Bash approval: the question, something that names it as a command, and an option list.
  {
    id = "bash_permission_prompt",
    state = "blocked",
    priority = 850,
    visible = "blocked",
    match = function(view)
      local lines = lower(view.lines)
      if not holds(lines, "do you want to proceed?") then
        return false
      end
      local named = holds_any(lines, {
        "bash command",
        "bash(",
        "contains expansion",
        "tab to amend",
        "ctrl+e to explain",
      })
      if not named then
        return false
      end
      return any_line(lines, function(line)
        return is_option(line, false)
      end)
    end,
  },

  -- Any other approval: the question and a numbered answer, below the last rule on the screen
  -- so that the same words scrolled up in the transcript do not count.
  --
  -- On the Claude Code we captured, this literal is the wrong literal: a Write approval asks
  -- "Do you want to create note.txt?", naming the thing, and never says "proceed" at all — so
  -- this rule does not fire on the one screen it was written for, and what catches it is the
  -- backstop at 300. It is kept because it is the manifest's, and another version may yet word
  -- it this way; it is not what is holding that screen up.
  {
    id = "generic_permission_prompt",
    state = "blocked",
    priority = 840,
    visible = "blocked",
    match = function(view)
      local after = lower(view.after_rule())
      return holds(after, "do you want to proceed?")
        and holds(after, "esc to cancel")
        and any_line(after, function(line)
          return is_option(line, true)
        end)
    end,
  },

  -- The backstop, from before Claude Code settled on the shapes above: anything that reads as a
  -- question left standing. Not a leftover — it is what catches a Write approval, whose wording
  -- the rule above does not know. No `visible`, because what it has found is words rather than a
  -- prompt it can point at — a screen this is the best anyone can say about is not evidence
  -- enough to release the hold above it.
  --
  -- It used to read `view.lines`, the whole screen, and ask only whether the word "yes" was
  -- somewhere on it. That was survivable while a pane had to be running `claude` by name; it is
  -- not survivable now that every pane that is not a shell is claimed, because seven files in
  -- this repository hold the phrase "do you want to" and the word "yes", and opening any of
  -- them would have chimed. So: the region is what is below the last rule, like its two
  -- neighbours, and the evidence is an answer you could *pick* — `❯ 1. Yes`, `  2. No` — rather
  -- than the word "yes" anywhere at all. That is a narrowing and not a proof: a line can be
  -- shaped like an answer without being one, and "No such file or directory" is.
  --
  -- The screen is still read whole for the one thing that makes this rule stand down, an empty
  -- prompt box, because a guard that only ever suppresses can afford to look everywhere.
  --
  -- And the screen has to have drawn a rule at all. `view.after_rule()` answers the whole screen
  -- when there is none, which is the right answer for the regions — but for the loosest rule in
  -- the set it means "every line of a pager" all over again. Claude Code draws a `─` above
  -- everything it asks about, including the Write approval this rule exists for; prose in a
  -- pager does not. The stricter rules above can skip this, and one of them has to: the captured
  -- trust dialog draws no rule and `live_blocked_form` is what catches it.
  {
    id = "legacy_no_prompt_blocker",
    state = "blocked",
    priority = 300,
    match = function(view)
      if any_line(lower(view.lines), is_bare_prompt) then
        return false
      end
      if
        not any_line(view.lines, function(line)
          return first_ink(line) == RULE
        end)
      then
        return false
      end
      local lines = lower(view.after_rule())
      local answerable = any_line(lines, function(line)
        return is_option(line, false)
      end)
      if answerable and (holds(lines, "do you want to") or holds(lines, "would you like to")) then
        return true
      end
      return holds_any(lines, {
        "waiting for permission",
        "do you want to allow this connection?",
        "tab to amend",
        "ctrl+e to explain",
        "review your answers",
      })
    end,
  },

  -- The title again, at the bottom: what Claude Code leaves in it when a turn is over.
  {
    id = "osc_title_idle",
    state = "idle",
    priority = 250,
    visible = "idle",
    match = function(view)
      return title_opens(view.title, function(code)
        return code == STAR
      end)
    end,
  },
}

-- What a pane has to be running for these rules to be declined outright.
--
-- The test is round this way because only one of the two sets can be written down. A shell is
-- one of a dozen programs, named the same everywhere and unchanged for decades; the ways to
-- start an agent are a private matter between a user and their `~/.config` — a wrapper function
-- around `systemd-inhibit`, `env`, `nohup`, `sudo`, `npx`, a `direnv`-shimmed launcher — and the
-- foreground process group's name is the wrapper's, not `claude`. Naming the programs that are
-- Claude Code turns the detector off for everyone who launches it through anything; naming the
-- programs that are certainly *not* Claude Code costs, at worst, a ruleset asked about a pane
-- running `vim`, where none of its rules match and the pane falls through anyway.
--
-- Measured, not guessed, on 2026-09-24 against Claude Code v2.1.281 on this machine, reading
-- the foreground process group's `/proc/<pid>/comm` off a pty master the way the host does:
-- `bash` for the bare shell, `claude` once it is started, and `bash` again the moment it is
-- quit. v2.1.281 installs as a single native executable, so `comm` is its own name — which is
-- what made an allow-list of `claude` and `node` look sufficient until a wrapped launch showed
-- up. What the measurement actually establishes is the half that still matters: quitting Claude
-- Code puts the shell's own name back, and that is the edge this table has to catch.
--
-- Without that edge, a pane that has ever run Claude Code is never unclaimed: `osc_title_idle`
-- fires off a `✳` title that nothing resets when the program exits, so the shell you go on
-- using reads as an idle agent forever — no `blocked` for an `rm -i`, and no chime.
--
-- Add your own, or take one away, the way you would any table:
--
--   require('neotmux.agents.claude').shells.osh = true
claude.shells = {
  bash = true,
  sh = true,
  dash = true,
  zsh = true,
  fish = true,
  ksh = true,
  mksh = true,
  csh = true,
  tcsh = true,
  ash = true,
  busybox = true,
  nu = true,
  elvish = true,
  xonsh = true,
}

-- A pane nothing could be read about is claimed rather than declined: `pane.info` is `/proc`,
-- which is Linux only and empty for a process that went away between the host asking and the
-- answer coming back. There the screen is the only evidence there is, and refusing it would
-- turn off the detector rather than narrow it.
--
-- And a pane the host has identified is the identified agent's, and a program another ruleset
-- names is that ruleset's: this one claims by elimination, so it would otherwise run first on
-- every other agent's screen and answer out of the words the two programs happen to share.
function claude.claims(info)
  if info ~= nil and info.agent ~= nil then
    return info.agent == "claude"
  end
  local command = info and info.command
  if command == nil then
    return true
  end
  if claude.shells[command] then
    return false
  end
  local name = require("neotmux.manifest").lookup_name(command)
  for _, rules in pairs(require("neotmux.detect").agents) do
    if rules ~= claude and rules.names ~= nil and rules.names[name] then
      return false
    end
  end
  return true
end

return claude

-- What a Claude Code screen means, as eleven rules.
--
-- Ported from herdr (herdrdev/herdr, Apache-2.0), `src/detect/manifests/claude.toml`. The ids
-- and the priorities are the manifest's on purpose: they are what lets a reader hold this file
-- against the TOML it came from and see whether the two still say the same thing.
--
-- Two of the manifest's rules are not ported. `background_agents_working` and
-- `background_mcp_task_working` read the shapes Claude Code draws for work it is running out of
-- sight, and nobody here has a screen of either to check a rule against — a rule written from a
-- pattern alone is one that only ever matches the fixture written from the same pattern.
--
-- The fixtures in `crates/neotmux-lua/tests/detect.rs` are written from that manifest and from
-- what is known of the current Claude Code UI, not from captured screens: they say what these
-- rules mean, and only the ones a real session has been held against say what Claude draws.
--
--   detect.agents = { claude = require('neotmux.agents.claude') }

-- Codepoints, not a pattern class: `✳` is three bytes, and `[✳✶✻]` is a class of those bytes
-- taken singly, which matches a byte of nearly every other symbol on the screen.
local SPINNER = {
  [0x2a] = true, -- *
  [0xb7] = true, -- ·
  [0x2722] = true, -- ✢
  [0x2733] = true, -- ✳
  [0x2736] = true, -- ✶
  [0x273b] = true, -- ✻
  [0x273d] = true, -- ✽
}
local PAUSE, PLAY = 0x23f8, 0x23f5 -- ⏸ ⏵
local BOTTOM = 0x2570 -- ╰
-- What the input box draws to the left of what you are typing. Two of them: which one it is
-- has changed between versions of Claude Code, and a rule betting on one goes blind on the other.
local PROMPT = { [0x3e] = true, [0x276f] = true } -- > ❯
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

-- `❯ 1. Yes`, with the box edge and the chevron cut off first: an option list is drawn inside
-- the dialog's box, and `❯` is three bytes, so `❯?` would make only the last of them optional.
-- `numbered` is for the rule that wants the option list rather than any line that happens to
-- begin with a yes.
local function is_option(line, numbered)
  local body = line:gsub("^%s*\u{2502}", ""):gsub("^%s*\u{276f}", "")
  local digit = numbered and "%d" or "%d?"
  return body:match("^%s*" .. digit .. "%.?%s*yes") ~= nil
    or body:match("^%s*" .. digit .. "%.?%s*no") ~= nil
end

-- A line that is only a chevron: the empty input box, and the one screen the legacy blocker
-- below must not fire on. The `│` edges are the box rather than what the line says.
local function is_bare_prompt(line)
  return line:match("^%s*\u{2502}?%s*\u{276f}%s*\u{2502}?%s*$") ~= nil
end

-- What is drawn *under* the last rounded box. `view.prompt_box()` is the box's body, and the
-- hint line that tells a dialog from an input box is below it — nothing for a box whose bottom
-- has not been drawn yet, because the body already runs to the end of the screen there.
local function under_box(lines)
  local from
  for i = #lines, 1, -1 do
    if first_ink(lines[i]) == BOTTOM then
      from = i + 1
      break
    end
  end
  local out = {}
  for i = from or (#lines + 1), #lines do
    out[#out + 1] = lines[i]
  end
  return out
end

return {
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
  {
    id = "btw_overlay_working",
    state = "working",
    priority = 975,
    visible = "working",
    match = function(view)
      return any_line(lower(view.tail(5)), function(line)
        return line:match("^%s*/btw") ~= nil or line:match("esc to close%s*$") ~= nil
      end)
    end,
  },

  -- A turn in progress: the interrupt hint against a paused or running marker, or a spinner
  -- line with its trailing ellipsis.
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
        return SPINNER[ink] ~= nil and line:find(ELLIPSIS, 1, true) ~= nil
      end)
    end,
  },

  -- The input box with nothing running behind it. A dialog is drawn in the same box and its
  -- selected option starts with the same character, so the exclusions below are what tell the
  -- two apart — and they read the lines *under* the box as well as its body, because that is
  -- where Claude puts the hint. herdr could exclude less: its prompt box was cut at a plain
  -- `───` rule, so a rounded dialog box was never the prompt box there and this rule could not
  -- reach one. Ours is the last rounded box, which is the wider region, and the manifest's
  -- priorities are only safe under it because of this list.
  {
    id = "live_prompt_box",
    state = "idle",
    priority = 950,
    visible = "idle",
    match = function(view)
      local box = lower(view.prompt_box())
      if not any_line(box, function(line)
        return PROMPT[first_ink(line)] ~= nil
      end) then
        return false
      end
      local asking = {
        "enter to select",
        "esc to cancel",
        "arrow keys",
        "\u{2191}/\u{2193} to navigate",
        -- A box putting a question to you is not an idle prompt, whatever its hint line says.
        "do you want to proceed?",
      }
      return not holds_any(box, asking) and not holds_any(lower(under_box(view.lines)), asking)
    end,
  },

  -- Picking a model is a menu over the top, not a state: the turn underneath it is whatever it
  -- was. Told apart from a permission prompt, which is drawn the same way and is a state.
  {
    id = "model_picker_menu",
    priority = 900,
    match = function(view)
      local lines = lower(view.lines)
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
  -- question left standing. No `visible`, because what it has found is words rather than a
  -- prompt it can point at — a screen this is the best anyone can say about is not evidence
  -- enough to release the hold above it.
  {
    id = "legacy_no_prompt_blocker",
    state = "blocked",
    priority = 300,
    match = function(view)
      local lines = lower(view.lines)
      if any_line(lines, is_bare_prompt) then
        return false
      end
      local answerable = holds(lines, "yes") or holds(lines, "\u{276f}")
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

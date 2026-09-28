-- herdr's manifest engine, so that herdr's agent manifests run here as they are written.
--
--   local manifest = require('neotmux.manifest')
--   local agents = require('neotmux.detect').agents
--   agents.codex = manifest.ruleset({ id = 'codex', names = { 'codex' }, rules = { ... } })
--
-- A manifest is data: rules with a region, a state, a priority and a gate of `contains`,
-- `regex`, `line_regex`, `all`, `any` and `not`. `ruleset` turns one into what
-- `neotmux.detect` runs, a rule table per rule with a `match` of its own. The files in
-- `neotmux/agents/` other than `claude.lua` are herdr's `src/detect/manifests/*.toml`, turned
-- into Lua by `tools/herdr-manifests.py` and not edited by hand.
--
-- A port of herdr's src/detect/manifest.rs (herdrdev/herdr, Apache-2.0): the regions, the gate
-- and the fallback are its. What is not here is what neotmux has no source for — `osc_progress`
-- is always empty, as herdr's own engine is when it has not seen one — and `contains` lowercases
-- ASCII only, where herdr lowercases Unicode.
local M = {}

-- What herdr answers for a pane it has identified and whose screen no rule recognises: the
-- agent is up and not visibly doing anything. Codex is the exception, below.
M.FALLBACK = "default_known_agent_idle_fallback"

-- Byte offsets, because herdr's regions are slices of one string and a pattern anchored at `\z`
-- or at a line break sees exactly where the slice ends.
local function split(text)
  local lines, starts = {}, {}
  if text == "" then
    return lines, starts
  end
  local at = 1
  while true do
    local nl = text:find("\n", at, true)
    starts[#starts + 1] = at
    if nl == nil then
      lines[#lines + 1] = text:sub(at)
      break
    end
    lines[#lines + 1] = text:sub(at, nl - 1)
    at = nl + 1
    -- Rust's `lines`: a final newline ends the last line rather than starting an empty one.
    if at > #text then
      break
    end
  end
  return lines, starts
end

-- Where line `index` (1-based) starts, or one past the end for an index past the last line.
local function start_of(text, starts, index)
  return starts[index] or #text + 1
end

local function from_line(text, starts, index)
  return text:sub(start_of(text, starts, index))
end

local function upto_line(text, starts, index)
  return text:sub(1, start_of(text, starts, index) - 1)
end

local function blank(line)
  return line:match("^%s*$") ~= nil
end

local RULE = "\u{2500}"

local function is_horizontal_rule(line)
  local trimmed = line:match("^%s*(.-)%s*$")
  if trimmed == "" then
    return false
  end
  local run, at = 0, 1
  while trimmed:sub(at, at + #RULE - 1) == RULE do
    run = run + 1
    at = at + #RULE
  end
  if run == 0 then
    return false
  end
  return trimmed:sub(at):match("^%s*$") ~= nil or run >= 3
end

local function codex_prompt_line(line)
  return line == "\u{203a}" or line:sub(1, #"\u{203a} ") == "\u{203a} "
end

local BLOCK_MARKERS = { "\u{2022}", "\u{25a0}", "\u{2717}", "\u{2713}" }

local function codex_block_marker_line(line)
  for _, marker in ipairs(BLOCK_MARKERS) do
    if line:sub(1, #marker) == marker then
      return true
    end
  end
  return false
end

local function last_prompt_index(lines)
  for i = #lines, 1, -1 do
    if codex_prompt_line(lines[i]) then
      return i
    end
  end
  return nil
end

local function current_prompt_index(lines)
  local index = last_prompt_index(lines)
  if index == nil then
    return nil
  end
  for i = index + 1, #lines do
    if codex_block_marker_line(lines[i]) then
      return nil
    end
  end
  return index
end

local function prompt_box_top(lines)
  local borders = 0
  for i = #lines, 1, -1 do
    if is_horizontal_rule(lines[i]) then
      borders = borders + 1
      if borders == 2 then
        return i
      end
    end
  end
  return nil
end

local function above_prompt_box(text, lines, starts)
  local top = prompt_box_top(lines)
  if top == nil then
    return text
  end
  return upto_line(text, starts, top)
end

local function last_non_empty(text)
  local lines = split(text)
  for i = #lines, 1, -1 do
    if not blank(lines[i]) then
      return lines[i]
    end
  end
  return ""
end

local named = {}

named.whole_recent = function(text)
  return text
end

named.after_last_prompt_marker = function(text, lines, starts)
  local index = last_prompt_index(lines)
  if index == nil then
    return text
  end
  return from_line(text, starts, index + 1)
end

named.before_current_prompt_marker = function(text, lines, starts)
  local index = current_prompt_index(lines)
  if index == nil then
    return text
  end
  return upto_line(text, starts, index)
end

named.whole_recent_without_current_prompt_marker = function(text, lines)
  if current_prompt_index(lines) ~= nil then
    return ""
  end
  return text
end

named.current_prompt_block_marker = function(_, lines)
  local index = current_prompt_index(lines)
  if index == nil then
    return ""
  end
  for i = index - 1, 1, -1 do
    if codex_block_marker_line(lines[i]) then
      return lines[i]
    end
  end
  return ""
end

named.after_current_prompt_block_marker = function(text, lines, starts)
  local index = current_prompt_index(lines)
  if index == nil then
    return ""
  end
  for i = index - 1, 1, -1 do
    if codex_block_marker_line(lines[i]) then
      return from_line(text, starts, i)
    end
  end
  return ""
end

named.prompt_box_body = function(text, lines, starts)
  local top = prompt_box_top(lines)
  if top == nil then
    return ""
  end
  local stop = #lines + 1
  for i = top + 1, #lines do
    if is_horizontal_rule(lines[i]) then
      stop = i
      break
    end
  end
  return text:sub(start_of(text, starts, top + 1), start_of(text, starts, stop) - 1)
end

named.above_prompt_box = above_prompt_box

named.last_non_empty_above_prompt_box = function(text, lines, starts)
  return last_non_empty(above_prompt_box(text, lines, starts))
end

named.after_last_horizontal_rule = function(text, lines, starts)
  local from = nil
  for i = 1, #lines do
    if is_horizontal_rule(lines[i]) then
      from = i + 1
    end
  end
  if from == nil then
    return text
  end
  return from_line(text, starts, from)
end

local counted = {}

counted.bottom_lines = function(text, lines, starts, count)
  return from_line(text, starts, math.max(1, #lines - count + 1))
end

counted.bottom_non_empty_lines = function(text, lines, starts, count)
  local found, first = 0, nil
  for i = #lines, 1, -1 do
    if found == count then
      break
    end
    if not blank(lines[i]) then
      found, first = found + 1, i
    end
  end
  if first == nil then
    return ""
  end
  return from_line(text, starts, first)
end

counted.top_non_empty_lines = function(text, lines, starts, count)
  local found, last = 0, nil
  for i = 1, #lines do
    if found == count then
      break
    end
    if not blank(lines[i]) then
      found, last = found + 1, i
    end
  end
  if last == nil then
    return ""
  end
  return upto_line(text, starts, last + 1)
end

-- A view's lines and where each starts, cut once per view: every rule of a ruleset reads the
-- same screen. Weak, so a view `neotmux.detect` has let go of goes with it.
local cuts = setmetatable({}, { __mode = "k" })

-- The region a rule reads, out of the view `neotmux.detect` built. An unknown name is the
-- empty string, as it is in herdr: a rule over a region nobody can cut matches nothing.
function M.region(view, spec)
  spec = spec:match("^%s*(.-)%s*$")
  if spec == "osc_title" then
    return view.title or ""
  elseif spec == "osc_progress" then
    return ""
  end
  local text = view.text()
  local cut = cuts[view]
  if cut == nil then
    local lines, starts = split(text)
    cut = { lines = lines, starts = starts }
    cuts[view] = cut
  end
  local lines, starts = cut.lines, cut.starts
  local named_cut = named[spec]
  if named_cut ~= nil then
    return named_cut(text, lines, starts)
  end
  local name, count = spec:match("^([%a_]+)%((%d+)%)$")
  if name ~= nil and counted[name] ~= nil and count:sub(1, 1) ~= "0" then
    return counted[name](text, lines, starts, tonumber(count))
  end
  return ""
end

-- herdr's `compiled_gate_matches`, in its order: every `contains`, every `regex`, every
-- `line_regex` on some line, every `all`, one of `any` when there are any, none of `not`.
local function gate(g, text, lower)
  for _, needle in ipairs(g.contains or {}) do
    if not lower:find(needle:lower(), 1, true) then
      return false
    end
  end
  for _, pattern in ipairs(g.regex or {}) do
    if not neotmux.regex.is_match(pattern, text) then
      return false
    end
  end
  if g.line_regex ~= nil and #g.line_regex > 0 then
    local lines = split(text)
    for _, pattern in ipairs(g.line_regex) do
      local hit = false
      for _, line in ipairs(lines) do
        if neotmux.regex.is_match(pattern, line) then
          hit = true
          break
        end
      end
      if not hit then
        return false
      end
    end
  end
  for _, nested in ipairs(g.all or {}) do
    if not gate(nested, text, lower) then
      return false
    end
  end
  if g.any ~= nil and #g.any > 0 then
    local one = false
    for _, nested in ipairs(g.any) do
      if gate(nested, text, lower) then
        one = true
        break
      end
    end
    if not one then
      return false
    end
  end
  for _, nested in ipairs(g["not"] or {}) do
    if gate(nested, text, lower) then
      return false
    end
  end
  return true
end

M.gate = gate

-- What a pane's command is called once it is a name herdr would look up: the basename, lower
-- case. `pane.info` answers `agent` too, herdr's reading of the whole foreground job — the
-- agent under a `node` or a shell — and a ruleset claims by that where the host could read it,
-- by the command's name where it could not.
local function lookup_name(command)
  return (command:match("([^/]+)$") or command):lower()
end

-- `state = 'unknown'` is herdr's word for "this screen is not a state", which here is a rule
-- with no state at all. `visible_*` counts only for the state it names, as it does in herdr.
local function rule_of(r)
  local state = r.state
  if state == "unknown" then
    state = nil
  end
  local visible = nil
  if r.visible_idle and state == "idle" then
    visible = "idle"
  elseif r.visible_blocker and state == "blocked" then
    visible = "blocked"
  elseif r.visible_working and state == "working" then
    visible = "working"
  end
  local region = r.region or "whole_recent"
  return {
    id = r.id,
    state = state,
    priority = r.priority or 0,
    visible = visible,
    gate = r,
    match = function(view)
      local text = M.region(view, region)
      return gate(r, text, text:lower())
    end,
  }
end

-- Compiles every pattern in a gate now, so that the first look at a pane does not: a
-- manifest's patterns take milliseconds to compile, a tick has eight, and a ruleset is built
-- while the config loads. A pattern that does not compile is left for its rule to raise.
local function compile(g)
  for _, field in ipairs({ "regex", "line_regex" }) do
    for _, pattern in ipairs(g[field] or {}) do
      pcall(neotmux.regex.is_match, pattern, "")
    end
  end
  for _, field in ipairs({ "all", "any", "not" }) do
    for _, nested in ipairs(g[field] or {}) do
      compile(nested)
    end
  end
end

-- The ruleset is the list of its rules, as `neotmux.detect` reads one, with `claims` and the
-- names it claims beside them. `names` is a set, so that another ruleset can ask whether a
-- command is this one's without calling it.
function M.ruleset(manifest)
  local set = {}
  for _, r in ipairs(manifest.rules) do
    compile(r)
    set[#set + 1] = rule_of(r)
  end
  -- Codex alone is left unanswered: herdr calls a Codex screen none of its rules recognise
  -- ambiguous rather than idle, and here that is the pane falling through to `neotmux.state`.
  if manifest.id ~= "codex" then
    set[#set + 1] = {
      id = M.FALLBACK,
      state = "idle",
      priority = math.mininteger,
      match = function()
        return true
      end,
    }
  end
  local names = {}
  for _, name in ipairs(manifest.names or {}) do
    names[name] = true
  end
  set.names = names
  function set.claims(info)
    if info == nil then
      return false
    end
    if info.agent ~= nil then
      return info.agent == manifest.id
    end
    return info.command ~= nil and names[lookup_name(info.command)] == true
  end
  return set
end

M.lookup_name = lookup_name

return M

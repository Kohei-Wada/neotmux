-- Two keys pick anything on the screen, the way Vimium and tmux-fingers do.
--
-- A terminal's output is full of things you want to act on — a url, a path, a hash — and the
-- only way to reach one has been to read it and type it back in. `neotmux.hints` finds every
-- one of them, labels each with a key or two, and answers with the one you typed. What to do
-- with it is the caller's:
--
--   local hints = require('neotmux.hints')
--   hints.open({
--     done = function(target) neotmux.pane.send(target.pane, target.text) end,
--   })
--
-- `panes` is which panes to look at — `'visible'`, the default, or an explicit list of ids.
-- `targets` narrows the built-in finders by name; `find = function(pane, lines) return { {
-- line =, col =, text = }, ... } end` replaces them outright with the caller's own — it is
-- called once per pane, with `lines` nil where that pane's content could not be read, since a
-- finder need not work from `lines` at all. `done` and
-- `cancel` are the caller's, called with `{ pane =, line =, col =, text = }` and with nothing,
-- respectively — `cancel` is optional, and Esc and `q` both reach it. `line` is the row within
-- the pane, zero-based, as `pane.cursor` counts; `col` is the column the target starts at,
-- counted with `neotmux.width` and never `#`, since a run can hold a wide character.
--
-- One hint session at a time, the same as `neotmux.list`: opening another cancels the first,
-- so the caller that opened it hears its own `cancel` rather than being left waiting on a
-- session that is gone. `open` returns a handle with `close()`, idempotent, and nil when there
-- was nothing on the screen to label — a session with no targets is never opened.
--
-- Every pane a target was found on is frozen for the life of the session and thawed on the
-- way out, whichever way it leaves — except a pane that was already frozen when hints opened
-- (copy mode, say, got there first): that one is left exactly as it was found.
--
-- `M.keys` is the home row a label is built from, read fresh every `open` so a user's own
-- `init.lua` can replace it before this ever runs. One character while there are few enough
-- targets to letter them singly, two when there are not; a target never gets a label another
-- target's label is a prefix of, which two-character labels get for free here since this
-- module never mixes the two lengths in one session. `Esc` and `q` are read before labels are,
-- so `M.keys` must not hold `q`: a target labelled with it could never be picked. Nearest targets get the shortest labels:
-- the focused pane first, then the row closest to the top of the screen. Past `#M.keys ^ 2`
-- targets there are no labels left to give out; the farthest ones are left unlabelled rather
-- than handed a label that collides with another's.
local modes = require("neotmux.mode")

local M = {}

M.colors = {
  bg = "#f9e2af", -- yellow
  fg = "#1e1e2e", -- mantle: readable on the yellow above
}

-- Read on every open, not cached here, so replacing it from an `init.lua` takes effect on the
-- next `hints.open()` without requiring this file to be reloaded.
M.keys = "asdfghjkl;"

-- `text` at the column each match starts at, on every occurrence `pattern` has in `line` —
-- columns, never bytes: `neotmux.width` of what comes before a match is what a cursor sees as
-- its start, and a byte offset would land inside a wide character instead of before it.
local function scan(line, pattern)
  local out = {}
  local pos = 1
  while true do
    local from, to = line:find(pattern, pos)
    if not from then
      break
    end
    out[#out + 1] = { col = neotmux.width(line:sub(1, from - 1)), text = line:sub(from, to) }
    pos = to + 1
  end
  return out
end

-- The built-in finders, each a pattern over one line. `M.finders` is exported so a user can
-- add to the table or replace an entry in it — `require('neotmux.hints').finders.url = ...` —
-- and `M.order` is the default set and the order they run in, since a plain `pairs` over a
-- table a user has added to would be an unspecified order.
M.order = { "url", "path", "hash", "ip" }

M.finders = {}

-- A scheme, `://`, and everything up to whitespace or a bracket that is more likely to be
-- closing a sentence around the url than part of it.
function M.finders.url(line)
  return scan(line, "%a[%w+.-]*://[^%s%)%]}>,]+")
end

-- An absolute path, a `./`- or `../`-relative one, or one under `~`. Three patterns rather
-- than one: what starts a path differs, but what it is made of afterwards does not. Each is
-- anchored to a word boundary with `%f[%S]`, so the `//` inside a url's `scheme://` is never
-- read as the start of an absolute path.
function M.finders.path(line)
  local out = {}
  for _, pattern in ipairs({
    "%f[%S]~/[%w_%.%-/]+",
    "%f[%S]%.%.?/[%w_%.%-/]+",
    "%f[%S]/[%w_%.%-/]+",
  }) do
    for _, hit in ipairs(scan(line, pattern)) do
      out[#out + 1] = hit
    end
  end
  return out
end

-- A run of 7 to 64 hex characters — a git object or a sha256, near enough. `%f[%x]` and
-- `%f[%X]` bound it to a maximal run, and the run is taken whole or not at all: a label that
-- answered with the first 40 characters of a 64-character hash would be a copy of something
-- that is not on the screen, which is worse than not labelling it.
function M.finders.hash(line)
  local out = {}
  for _, hit in ipairs(scan(line, "%f[%x]%x+%f[%X]")) do
    if #hit.text >= 7 and #hit.text <= 64 then
      out[#out + 1] = hit
    end
  end
  return out
end

-- Four dotted numbers, optionally with `:port`.
function M.finders.ip(line)
  return scan(line, "%d+%.%d+%.%d+%.%d+:?%d*")
end

-- One target where two or more finders read the same stretch of the line — `/home/user/abc123`
-- is a whole path to `path` and a hash to `hash`, both true, but a person picking between them
-- should not have to. Longest span wins; anything another kept hit's span fully contains is
-- dropped, so the label goes on the one that reads as "the whole thing" rather than a piece of
-- it. Two spans of the same length that do not contain one another — different stretches of
-- the line — both keep their target.
local function drop_contained(hits)
  local by_length = {}
  for i, hit in ipairs(hits) do
    by_length[i] = hit
  end
  table.sort(by_length, function(a, b)
    return neotmux.width(a.text) > neotmux.width(b.text)
  end)
  local kept = {}
  for _, hit in ipairs(by_length) do
    local start = hit.col
    local stop = hit.col + neotmux.width(hit.text)
    local contained = false
    for _, k in ipairs(kept) do
      if start >= k.start and stop <= k.stop then
        contained = true
        break
      end
    end
    if not contained then
      kept[#kept + 1] = { start = start, stop = stop, hit = hit }
    end
  end
  local out = {}
  for _, k in ipairs(kept) do
    out[#out + 1] = k.hit
  end
  table.sort(out, function(a, b)
    return a.col < b.col
  end)
  return out
end

-- The state of the one session that may be open, and nil the rest of the time. `state.generation`
-- names which `open()` this state belongs to, the same as `neotmux.list`'s.
local state = nil
local mode_handle = nil
local generation = 0
local registered = false

local function close()
  if state then
    for pane in pairs(state.frozen) do
      neotmux.pane.unfreeze(pane)
    end
  end
  if mode_handle then
    mode_handle:lower()
    mode_handle = nil
  end
  state = nil
end

-- Closes a session whose caller's code just raised, and says which callback it was rather
-- than leaving the labels stuck on screen with nothing able to reach them: `find`, `done` and
-- `cancel` are the caller's own code, not this module's.
local function fail(label, err)
  close()
  neotmux.notify("error", "neotmux.hints: " .. label .. ": " .. tostring(err))
end

-- The finders named in `names`, in that order, skipping one a name does not resolve to —
-- `M.finders` is a table a user may have added to or trimmed.
local function finders_for(names)
  local list = {}
  for _, name in ipairs(names) do
    if M.finders[name] then
      list[#list + 1] = M.finders[name]
    end
  end
  return list
end

-- Every target on the panes `opts` names, unlabelled and unordered. `nil` and a callback name
-- and its error is what a raising `find` answers with, so the caller can `fail` it the same
-- way every other guarded callback here is failed.
local function gather(opts)
  local pane_ids
  if opts.panes == nil or opts.panes == "visible" then
    pane_ids = neotmux.pane.list()
  else
    pane_ids = opts.panes
  end

  local finder_fns
  if not opts.find then
    finder_fns = finders_for(opts.targets or M.order)
  end

  local found = {}
  for _, pane in ipairs(pane_ids) do
    local lines = neotmux.pane.content(pane)
    if opts.find then
      -- Called whether or not this pane's content could be read: a caller's finder is not
      -- obliged to work from `lines` at all — `plugin.copy`'s reads its own snapshot — and
      -- skipping it here would open a session with no targets on a pane that has plenty.
      local ok, hits = pcall(opts.find, pane, lines)
      if not ok then
        return nil, "find", hits
      end
      -- Grouped by row before the containment filter, which compares columns and so only
      -- means anything within one line.
      local by_line = {}
      local rows = {}
      for _, hit in ipairs(hits or {}) do
        if by_line[hit.line] == nil then
          by_line[hit.line] = {}
          rows[#rows + 1] = hit.line
        end
        local row = by_line[hit.line]
        row[#row + 1] = hit
      end
      table.sort(rows)
      for _, row in ipairs(rows) do
        for _, hit in ipairs(drop_contained(by_line[row])) do
          found[#found + 1] = { pane = pane, line = row, col = hit.col, text = hit.text }
        end
      end
    elseif lines then
      for row, line in ipairs(lines) do
        local hits = {}
        for _, finder in ipairs(finder_fns) do
          local ok, found_hits = pcall(finder, line)
          if not ok then
            return nil, "find", found_hits
          end
          for _, hit in ipairs(found_hits or {}) do
            hits[#hits + 1] = hit
          end
        end
        for _, hit in ipairs(drop_contained(hits)) do
          found[#found + 1] = { pane = pane, line = row - 1, col = hit.col, text = hit.text }
        end
      end
    end
  end
  return found
end

-- The nearest targets first: the focused pane's own, then everything else by how close to the
-- top of the screen it sits — `neotmux.pane.rect`, not `ctx:pane_rect`, because ordering
-- happens once, at `open`, before there has necessarily been a tick to ask a `ctx` about.
local function order(found, focus)
  local rects = {}
  local function top_of(pane)
    if rects[pane] == nil then
      rects[pane] = neotmux.pane.rect(pane) or false
    end
    local rect = rects[pane]
    return rect and rect.y or 0
  end
  table.sort(found, function(a, b)
    local a_focus = a.pane == focus
    local b_focus = b.pane == focus
    if a_focus ~= b_focus then
      return a_focus
    end
    local ay = top_of(a.pane) + a.line
    local by = top_of(b.pane) + b.line
    if ay ~= by then
      return ay < by
    end
    return a.col < b.col
  end)
end

-- One label per target, in the order given — the nearest target gets the shortest and
-- earliest one. One character each while `M.keys` has enough of them to go around; two when
-- it does not, which is also what keeps a label from ever being a prefix of another: every
-- label in a session is the same length as every other.
local function assign_labels(count)
  local chars = {}
  for i = 1, #M.keys do
    chars[i] = M.keys:sub(i, i)
  end
  local labels = {}
  if count <= #chars then
    for i = 1, count do
      labels[i] = chars[i]
    end
    return labels
  end
  local want = math.min(count, #chars * #chars)
  local at = 0
  for _, a in ipairs(chars) do
    for _, b in ipairs(chars) do
      if at >= want then
        break
      end
      at = at + 1
      labels[at] = a .. b
    end
  end
  return labels
end

-- No label left to type is a prefix of, so a raised key that could still complete more than
-- one label keeps the session waiting rather than picking either of them early.
local function has_prefix(prefix)
  for label in pairs(state.by_label) do
    if label:sub(1, #prefix) == prefix then
      return true
    end
  end
  return false
end

local function finish_pick(hit)
  local done = state.done
  close()
  if done then
    local ok, err = pcall(done, { pane = hit.pane, line = hit.line, col = hit.col, text = hit.text })
    if not ok then
      fail("done", err)
    end
  end
end

local function finish_cancel()
  local cancel = state.cancel
  close()
  if cancel then
    local ok, err = pcall(cancel)
    if not ok then
      fail("cancel", err)
    end
  end
end

-- One printable key at a time, joined to what has been typed since the last one landed. An
-- exact label picks its target; a prefix of one or more labels keeps waiting; anything else
-- starts over as though this key were the first one pressed, which is what lets a mistyped
-- first character be corrected by simply typing the right one next.
local function handle_type(key)
  local typed = state.typed .. key
  local exact = state.by_label[typed]
  if exact then
    finish_pick(exact)
    return
  end
  if has_prefix(typed) then
    state.typed = typed
    return
  end
  local solo = state.by_label[key]
  if solo then
    finish_pick(solo)
    return
  end
  state.typed = has_prefix(key) and key or ""
end

local function arm()
  if registered then
    return
  end
  registered = true

  neotmux.on_key(function(key)
    if state == nil then
      return false
    end
    if key == "Esc" or key == "q" then
      finish_cancel()
      return true
    end
    if utf8.len(key) == 1 then
      handle_type(key)
    end
    return true
  end)

  neotmux.on_tick(function(ctx)
    if state == nil then
      return
    end
    for _, hit in ipairs(state.targets) do
      local rect = ctx:pane_rect(hit.pane)
      if rect then
        -- Held inside the pane: a two-character label on a target that starts in the last
        -- column would otherwise be drawn over the border, or over the pane next door.
        local room = rect.w - neotmux.width(hit.label)
        local col = math.max(0, math.min(hit.col, room))
        ctx:text(nil, rect.x + col, rect.y + hit.line, hit.label, {
          fg = M.colors.fg,
          bg = M.colors.bg,
        })
      end
    end
  end)
end

local function handle_for(my_generation)
  local function bound()
    return state ~= nil and state.generation == my_generation
  end
  local handle = {}
  function handle:close()
    if bound() then
      close()
    end
  end
  return handle
end

-- Opens a hint session, closing one already up. See the module header for the shape of
-- `opts`.
function M.open(opts)
  -- Cancelled, not merely closed: the caller of the session being pre-empted has no other way
  -- to learn that its pick is over, and one that yields the keyboard while a pick is up — as
  -- `plugin.copy` does — would yield it forever.
  if state then
    finish_cancel()
  end
  opts = opts or {}

  local found, label, err = gather(opts)
  if found == nil then
    fail(label, err)
    return nil
  end

  if #found == 0 then
    neotmux.notify("info", "nothing to hint")
    return nil
  end

  order(found, neotmux.pane.current())

  local labels = assign_labels(#found)
  for i = #found, #labels + 1, -1 do
    table.remove(found, i)
  end

  local by_label = {}
  for i, hit in ipairs(found) do
    hit.label = labels[i]
    by_label[hit.label] = hit
  end

  -- Frozen once per pane, and only where it was not already: copy mode, or an earlier hint
  -- session, may have gotten there first, and this session must not be the one that thaws it.
  -- `freeze` answers `true` for any pane that exists, frozen or not already — it is `frozen`
  -- that says which, and it has to be asked before `freeze` runs and makes the question moot.
  local frozen = {}
  local seen = {}
  for _, hit in ipairs(found) do
    if not seen[hit.pane] then
      seen[hit.pane] = true
      local already = neotmux.pane.frozen(hit.pane)
      if not already and neotmux.pane.freeze(hit.pane) then
        frozen[hit.pane] = true
      end
    end
  end

  generation = generation + 1
  local my_generation = generation
  state = {
    generation = my_generation,
    targets = found,
    by_label = by_label,
    typed = "",
    frozen = frozen,
    done = opts.done,
    cancel = opts.cancel,
  }
  mode_handle = modes.raise("hints")
  arm()
  return handle_for(my_generation)
end

return M

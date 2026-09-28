-- A keyboard-driven list in a float, in the same spirit as `neotmux.box` and
-- `neotmux.prompt`.
--
-- The session picker and the sidebar each grew their own cursor, their own scrolling and
-- their own row drawing. This is those parts with the data taken out: a float, a cursor `j`
-- and `k` move and wrap, a window that follows the cursor the way `plugin.herdr.sidebar` scrolls,
-- and — where the caller wants one — an input line for a filter or for `ask`. The host has
-- never heard of any of this; it is `neotmux.float`, `neotmux.box`, `neotmux.mode` and an
-- `on_key`/`on_tick` over a local, the same shape every convention module here is built from.
--
--   local list = require('neotmux.list')
--   local handle = list.open({
--     title = 'sessions',
--     items = function() return neotmux.session.list() end,
--     row = function(item, chosen, width) return { { text = item } } end,
--     done = function(item, handle) neotmux.session.switch(item) end,
--   })
--
-- One list is open at a time: opening another closes the first, the same as `neotmux.prompt`.
-- A stack of lists has no consumer here. `on_key` and `on_tick` are armed the first time a
-- list opens, the way `neotmux.guard` arms itself — a layer that requires this file and never
-- opens a list holds no subscription for it.
--
-- `width` is columns, or — at one or less — the share of the pane area to take, so a palette
-- can ask for four fifths of whatever the terminal is.
--
-- `place` is where the window hangs: `'center'` over the pane area, which is the default and
-- what a picker wants, or `'top'`, which drops it from the top edge and moves the input line to
-- the first row — a command palette, where what you type is what you look at. `empty` is a line
-- to draw when nothing is left to show, so a query that matched nothing says so rather than
-- leaving a frame with nothing in it. `header` is `function(width) return runs end`, a row
-- drawn above the items that the cursor never lands on — the column names of a table.
--
-- An input line — a query or an `ask` — takes the editing keys a line of text takes anywhere,
-- `neotmux.line`'s: `Backspace` and `C-h` take the last character back, `C-w` the last word,
-- and `C-u` clears the line.
--
-- `input = true` turns the line on: every printable key joins a query,
-- `C-j`/`C-k` move the cursor instead of the plain `j`/`k`, and `handle:query()` reads what
-- has been typed; `query` is what it starts with, typed in already. `filter` is a separate,
-- optional predicate — `function(item, query) return ... end` — that narrows `items()` by that
-- query. A list can take `input` with no `filter`
-- (the query is there to read; nothing narrows because of it — see `plugin.palette`, which
-- ranks inside `items` itself against `handle:query()`) or `filter` with no `input`, which then
-- does nothing on its own: there is no query for it to be handed.
--
-- A few combinations are quiet rather than refused, worth knowing rather than discovering:
-- `input` takes the last row, so `footer` never draws while it is on; a key claimed in `keys`
-- steals that character away from the query instead of it being typed; `q` is only a cancel
-- when there is no input line, since a filtered list needs the letter as text; and while `ask`
-- is up it replaces and freezes the query line, so nothing typed then reaches `filter`.
--
-- `items`, `row`, `header`, `filter`, `footer`, `done`, `cancel`, a `keys` entry, and `ask`'s own
-- `accept`/`done`/`cancel` are all the caller's code, charged to this module's own tick budget
-- — `neotmux/list` — so a slow one parks every list in the process until
-- `neotmux.resubscribe('neotmux/list')` restores it. Every one of those calls is guarded: a
-- raise closes the list and says which callback it was through `neotmux.notify`, rather than
-- leaving the float frozen on its last drawn frame (a raise on the tick) or letting the key
-- that triggered it fall through to the pane behind the float (a raise on a key). Esc itself
-- is checked before any of that for the list itself, so it closes even when every one of them
-- is broken; while `ask` is up, Esc dismisses the input line first and only then runs
-- `cancel` — guarded the same as everything else — so a raise in it cannot leak the key that
-- dismissed it.
local modes = require("neotmux.mode")
local box = require("neotmux.box")
local line = require("neotmux.line")
local style = require("neotmux.style")
local g = neotmux.group("neotmux.list")

local M = {}

style.default("float.list", {
  frame = "mantle",
  text = "text",
  cursor = "surface0", -- the row under the caret
  caret = "yellow", -- the block in an input line
  hint = "surface2",
})

-- The state of the one list that may be open, and nil the rest of the time. `state.generation`
-- names which `open()` this state belongs to, so a handle from an earlier open can tell that
-- the list it names is gone even once a later one is up in its place — see `bound`.
local state = nil
local win = nil
local mode_handle = nil
local generation = 0

-- Where the panes are, in surface coordinates, as of the last tick: the list is centred over
-- them rather than over the terminal, so it does not sit on the status bar or a sidebar. It is
-- remembered rather than asked for, because a key is pressed between two ticks and `ctx`
-- belongs to a tick.
local area = nil

local function close()
  if win then
    win:remove()
    win = nil
  end
  if mode_handle then
    mode_handle:lower()
    mode_handle = nil
  end
  state = nil
end

-- Closes a list whose caller's code just raised, and says which callback it was rather than
-- leaving the float stuck open with nothing able to reach it: `items`, `row` and the rest are
-- the caller's own code, not this module's, and a bug in one of them is not a reason a person
-- can no longer press Esc.
local function fail(label, err)
  close()
  neotmux.notify("error", "neotmux.list: " .. label .. ": " .. tostring(err))
end

-- `text` cut to at most `room` columns, on a character boundary — `neotmux.width`, never `#`,
-- since a run can hold a wide character and a byte count would cut one in half.
local function truncate(text, room)
  if neotmux.width(text) <= room then
    return text
  end
  local out = ""
  for _, code in utf8.codes(text) do
    local ch = utf8.char(code)
    if neotmux.width(out .. ch) > room then
      break
    end
    out = out .. ch
  end
  return out
end

-- Draws one row of runs, left to right from `x`, clipped to `limit` columns rather than
-- allowed to run past the frame into somebody else's cells. Columns are `neotmux.width`,
-- never `#`: a run can hold a wide character. A run longer than the room left is truncated to
-- it, not merely refused — the frame is the caller's, not a run's to paint over.
local function draw_runs(ctx, target, x, y, limit, runs, bg)
  local at = x
  for _, run in ipairs(runs) do
    local room = limit - (at - x)
    if room <= 0 then
      break
    end
    local text = truncate(run.text or "", room)
    ctx:text(target, at, y, text, { fg = run.fg, bg = run.bg or bg })
    at = at + neotmux.width(text)
  end
end

-- The rows the window shows, taken fresh: an item can appear or go while the list is up, and
-- the cursor is clamped to whatever is there now rather than to what was there on the way in.
local function refresh()
  local items = state.opts.items() or {}
  if state.query and state.opts.filter then
    local filter = state.opts.filter
    local kept = {}
    for _, item in ipairs(items) do
      if filter(item, state.query) then
        kept[#kept + 1] = item
      end
    end
    items = kept
  end
  state.items = items
  if #state.items == 0 then
    state.at = 1
  elseif state.at > #state.items then
    state.at = #state.items
  elseif state.at < 1 then
    state.at = 1
  end
end

-- Which rows are on screen, the window following the cursor as little as it can — the rule
-- `plugin.herdr.sidebar` scrolls by: the cursor never leaves the screen, and the top only moves
-- exactly as far as it has to to keep it there.
local function window_over(count, rows, at)
  if count <= rows then
    return 1
  end
  local first = state.first or 1
  if at < first then
    first = at
  elseif at > first + rows - 1 then
    first = at - rows + 1
  end
  first = math.max(1, math.min(first, count - rows + 1))
  return first
end

-- `M.commands` is exported so `M.keys` can name an entry by string, the same shape
-- `plugin.tmux.copy` uses — but nothing stops it being called with no list open, so every entry
-- is a no-op then rather than indexing `state` when it is `nil`.
M.commands = {
  ["down"] = function()
    if not state then
      return
    end
    state.at = state.at + 1
    if state.at > #state.items then
      state.at = 1
    end
  end,
  ["up"] = function()
    if not state then
      return
    end
    state.at = state.at - 1
    if state.at < 1 then
      state.at = #state.items
    end
  end,
  ["choose"] = function()
    if not state then
      return
    end
    local item = state.items[state.at]
    if item and state.opts.done then
      local ok, err = pcall(state.opts.done, item, state.handle)
      if not ok then
        fail("done", err)
      end
    end
  end,
  ["cancel"] = function()
    if not state then
      return
    end
    local cancel = state.opts.cancel
    close()
    if cancel then
      local ok, err = pcall(cancel)
      if not ok then
        fail("cancel", err)
      end
    end
  end,
}

-- `ask`'s own keys: every printable character, and three that are not. Filtering does not
-- come through here — it stays live under the ordinary keys below, since a filtered list is
-- still a list a person moves the cursor in (with `C-j`/`C-k`; see the key handler).
local function typing(key)
  local input = state.input
  if key == "Esc" then
    -- The overlay is gone before `cancel` ever runs, so a raise in it cannot undo the
    -- dismissal — only the guard below has anything left to do.
    state.input = nil
    if input.cancel then
      local ok, err = pcall(input.cancel)
      if not ok then
        fail("ask cancel", err)
      end
    end
  elseif key == "Enter" then
    local value = input.value
    state.input = nil
    if input.done then
      local ok, err = pcall(input.done, value)
      if not ok then
        fail("ask done", err)
      end
    end
  else
    -- `accept`, when given, is asked before a character joins the buffer: the picker's
    -- refusal of a leading `-` is "no" here and nowhere else. Guarded like every callback: a
    -- raise is a refusal, and says so.
    local accept = input.accept
      and function(char, value)
        local ok, accepted = pcall(input.accept, char, value)
        if not ok then
          fail("ask accept", accepted)
          return false
        end
        return accepted
      end
    input.value = line.edit(input.value, key, accept) or input.value
  end
end

-- What a query line is introduced by. One glyph and a space, so the caret sits where the next
-- character will land.
M.prompt = "> "

-- How far down a `place = 'top'` list hangs from the top of the pane area. Far enough to read
-- as a thing that dropped down rather than a thing that was always there.
M.drop = 2

-- How big the window wants to be and where: the rows, the frame's own two, and a footer or
-- input line.
--
-- `place` is where it hangs. `'center'` is the middle of the pane area, which is what a picker
-- wants — you are choosing between things, and the middle is where your eyes are. `'top'`
-- drops it from the top edge instead, and puts the query line first rather than last, which is
-- what a command palette wants: you are typing, and what you type should be where you look.
local function wanted()
  -- A list with nothing in it still needs the row its `empty` line is drawn on, or the line
  -- lands on the frame.
  local shown = math.max(#state.items, state.opts.empty and 1 or 0)
  local rows = math.min(shown, state.opts.rows) + 3 + (state.opts.header and 1 or 0)
  -- A width of one or less is a share of the pane area rather than a column count, so a list
  -- can ask to be most of the screen without knowing how wide the screen is. Where no pane has
  -- a rect yet — nothing has been laid out — there is no area to take a share of, so the share
  -- is taken against the width a terminal has when nobody has said otherwise, and the next tick
  -- fits it to the real one.
  local room = area and area.w
  local want = state.opts.width
  if want <= 1 then
    want = math.max(1, math.floor((room or 80) * want))
  end
  local w = room and math.min(want, room) or want
  local h = math.min(rows, area and area.h or rows)
  local x = (area and area.x or 0) + math.max(0, math.floor(((area and area.w or w) - w) / 2))
  local y
  if state.opts.place == "top" then
    y = (area and area.y or 0) + math.min(M.drop, math.max(0, (area and area.h or h) - h))
  else
    y = (area and area.y or 0) + math.max(0, math.floor(((area and area.h or h) - h) / 2))
  end
  return { x = x, y = y, w = w, h = h }
end

-- Which row the input line takes, and which rows the items get. A `'top'` list reads downwards
-- from what you typed; every other list keeps the line at the bottom, where a picker's name
-- field has always been.
local function input_first()
  return state.opts.place == "top"
end

-- The rect every pane together covers, bands and all, read through `rect_of`, or nil when no pane
-- has one. The tick reads it through `ctx:pane_outer`; `open` reads the same geometry through
-- `neotmux.pane.outer`, because the tick that would otherwise tell it is armed by that open, and a
-- list placed before the area is known is drawn in the corner for a frame.
local function pane_area(rect_of)
  local left, top, right, bottom
  for _, pane in ipairs(neotmux.pane.list()) do
    local r = rect_of(pane)
    if r then
      left = math.min(left or r.x, r.x)
      top = math.min(top or r.y, r.y)
      right = math.max(right or (r.x + r.w), r.x + r.w)
      bottom = math.max(bottom or (r.y + r.h), r.y + r.h)
    end
  end
  if left then
    return { x = left, y = top, w = right - left, h = bottom - top }
  end
end

local registered = false

local function arm()
  if registered then
    return
  end
  registered = true

  g.on_key(function(key)
    if state == nil then
      return false
    end
    if state.input then
      typing(key)
      return true
    end
    -- Esc is handled before `refresh()` — before any caller code runs at all — so a list is
    -- always dismissable even when `items()` is the thing that is broken.
    if key == "Esc" then
      M.commands.cancel()
      return true
    end
    local ok, err = pcall(refresh)
    if not ok then
      fail("items", err)
      return true
    end
    if key == "Enter" then
      M.commands.choose()
      return true
    end
    local extra = state.opts.keys[key]
    if extra then
      local ok, err = pcall(extra, state.items[state.at], state.handle)
      if not ok then
        fail("a key", err)
      end
      return true
    end
    if state.opts.input then
      -- A list with `input` on is typed into rather than moved through by letter: `j` and `k`
      -- are characters a query may hold, so movement while typing is `C-j`/`C-k` instead —
      -- nothing above claims those, and nothing here changes what an `input`-less list does
      -- with a plain `j`/`k`.
      if key == "C-j" then
        M.commands.down()
        return true
      end
      if key == "C-k" then
        M.commands.up()
        return true
      end
      local edited = line.edit(state.query, key)
      if edited ~= nil then
        -- A query cleared starts the list again from the top.
        if edited == "" then
          state.at = 1
        end
        state.query = edited
      end
      return true
    end
    local vim = M.keys[key]
    local command = vim and M.commands[vim]
    if command then
      command()
    end
    -- A key nothing above claims is swallowed while a list is up: a list that let it through
    -- would type into the pane behind it.
    return true
  end)

  g.on_tick(function(ctx)
    area = pane_area(function(pane)
      return ctx:pane_outer(pane)
    end) or area

    if state == nil or win == nil then
      return
    end
    local c = style.get("float.list")
    local ok, err = pcall(refresh)
    if not ok then
      fail("items", err)
      return
    end
    local rect = ctx:float_rect(win)
    if rect == nil then
      return
    end
    local want = wanted()
    if rect.w ~= want.w or rect.h ~= want.h then
      win:resize_to(want.w, want.h)
    end
    if rect.x ~= want.x or rect.y ~= want.y then
      win:move_to(want.x, want.y)
    end
    local inner = box.draw(ctx, win, rect, { title = state.opts.title, bg = c.frame })

    local room = inner.h - 1
    local first_row = input_first() and inner.y + 1 or inner.y
    if state.opts.header then
      local ok, runs = pcall(state.opts.header, inner.w)
      if not ok then
        fail("header", runs)
        return
      end
      draw_runs(ctx, win, inner.x, first_row, inner.w, runs or {}, c.frame)
      room = room - 1
      first_row = first_row + 1
    end
    state.first = window_over(#state.items, room, state.at)
    for slot = 0, math.min(#state.items, room) - 1 do
      local at = state.first + slot
      local item = state.items[at]
      local row = first_row + slot
      local chosen = at == state.at
      local background = chosen and c.cursor or c.frame
      if chosen then
        ctx:rect(win, inner.x, row, inner.w, 1, { bg = c.cursor })
      end
      local ok, runs = pcall(state.opts.row, item, chosen, inner.w)
      if not ok then
        fail("row", runs)
        return
      end
      draw_runs(ctx, win, inner.x, row, inner.w, runs or {}, background)
    end

    local last = input_first() and inner.y or inner.y + inner.h - 1
    if state.input then
      local label = (state.input.label and state.input.label .. "  ") or ""
      ctx:text(win, inner.x, last, label .. state.input.value, {
        fg = c.text,
        bg = c.frame,
      })
      -- The caret, drawn rather than placed: the terminal's own cursor belongs to the focused
      -- pane, and a float cannot take it.
      ctx:rect(
        win,
        inner.x + neotmux.width(label) + neotmux.width(state.input.value),
        last,
        1,
        1,
        { bg = c.caret }
      )
    elseif state.opts.input then
      ctx:text(win, inner.x, last, M.prompt .. (state.query or ""), {
        fg = c.text,
        bg = c.frame,
      })
      ctx:rect(
        win,
        inner.x + neotmux.width(M.prompt .. (state.query or "")),
        last,
        1,
        1,
        { bg = c.caret }
      )
    elseif state.opts.footer then
      local ok, text = pcall(state.opts.footer)
      if not ok then
        fail("footer", text)
        return
      end
      if text then
        ctx:text(win, inner.x, last, text, { fg = c.hint, bg = c.frame })
      end
    end

    -- An empty frame says nothing about why it is empty. A query that matched none of them is
    -- the only way a list with items behind it can be empty, so that is what it says.
    if #state.items == 0 and state.opts.empty then
      ctx:text(win, inner.x, first_row, state.opts.empty, { fg = c.hint, bg = c.frame })
    end
  end)
end

-- `j`/`k` and nothing else: every mode neotmux ships moves with the vim four, so a hand that
-- has learned one has learned all of them. `q` is with them here rather than beside `Esc` in
-- the key handler, because a list with `input` on gives `q` back to the query text — the
-- search is what the letter belongs to once there is one to type into.
M.keys = {
  ["j"] = "down",
  ["k"] = "up",
  ["q"] = "cancel",
}

-- Builds the handle for the list just opened, bound to `my_generation` so that a callback
-- still holding it after a second list has opened over it finds nothing rather than reaching
-- into whatever is there now. `close`, like every handle in this ABI, is idempotent — on a
-- handle whose list is already gone, one way or another, it does nothing.
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
  function handle:selected()
    if not bound() then
      return nil
    end
    return state.items[state.at]
  end
  function handle:ask(ask_opts)
    if not bound() then
      return
    end
    state.input = {
      label = ask_opts.label,
      value = ask_opts.value or "",
      done = ask_opts.done,
      cancel = ask_opts.cancel,
      accept = ask_opts.accept,
    }
  end
  function handle:query()
    if not bound() then
      return nil
    end
    return state.query
  end
  return handle
end

-- Opens a list, closing one already up. See the module header for the shape of `opts`.
function M.open(opts)
  close()
  generation = generation + 1
  local my_generation = generation
  state = {
    generation = my_generation,
    at = 1,
    items = {},
    first = 1,
    input = nil,
    query = opts.input and (opts.query or "") or nil,
    opts = {
      title = opts.title,
      width = opts.width or 40,
      rows = opts.rows or 10,
      place = opts.place or "center",
      empty = opts.empty,
      items = opts.items,
      row = opts.row,
      header = opts.header,
      footer = opts.footer,
      keys = opts.keys or {},
      done = opts.done,
      cancel = opts.cancel,
      filter = opts.filter,
      input = opts.input,
    },
  }
  local ok, err = pcall(refresh)
  if not ok then
    fail("items", err)
    return nil
  end
  area = pane_area(neotmux.pane.outer) or area
  local float = g.float(wanted())
  if not float then
    state = nil
    return nil
  end
  win = float
  mode_handle = modes.raise(opts.mode or "list")

  local handle = handle_for(my_generation)
  state.handle = handle
  arm()
  return handle
end

return M

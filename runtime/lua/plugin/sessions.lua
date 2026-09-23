-- The session picker: every session there is, which one you are in, what each is running —
-- and a way to make one, go to one, or end one.
--
--   C-b s       open it
--   j k         the cursor, one row
--   Enter       go to the session under the cursor
--   c           make one: type a name, Enter to make it and go there, Esc to think again
--   d           end the one under the cursor
--   Esc q       leave
--
-- Nothing here is a host feature. The list is `neotmux.session.list()`, what each session is
-- running is `neotmux.session.with` over `pane.info`, the window is a `neotmux.float` with
-- `neotmux.box` drawn round it, and the keyboard is an `on_key` over a local — the same shape
-- copy mode is. The name field is drawn rather than typed into: the terminal's own cursor
-- belongs to the focused pane, so the block under the caret is a `ctx:rect`, which is what a
-- text field is anywhere else too.
local modes = require("neotmux.mode")
local box = require("neotmux.box")
local pane_state = require("neotmux.state")

local M = {}

-- Read every tick, so assigning to one from your own `init.lua` takes effect.
M.colors = {
  frame = "#181825", -- mantle, the same ground the status bar and the sidebar sit on
  name = "#cdd6f4",
  current = "#a6e3a1", -- green: the session you are in
  working = "#a6e3a1", -- green
  blocked = "#f9e2af", -- yellow: the one that wants you
  idle = "#585b70", -- surface2
  doing = "#6c7086", -- overlay0: what it is running, quieter than its name
  cursor = "#313244", -- surface0: the row under the caret, dark enough to read over
  caret = "#f9e2af", -- yellow, for the block in the name field
  hint = "#585b70", -- surface2: the key line, quieter than anything it describes
}

-- `j`/`k` and nothing else: every mode neotmux ships moves with the vim four, so a hand that
-- has learned one has learned all of them.
M.keys = {
  ["j"] = "down",
  ["k"] = "up",
  ["Enter"] = "choose",
  ["c"] = "create",
  ["d"] = "kill",
  ["Esc"] = "cancel",
  ["q"] = "cancel",
}

-- How wide the window is, and the most rows of it the list may take. Both are read on the way
-- in, so setting them from your own `init.lua` takes effect at the next open.
M.width = 40
M.rows = 10

-- The picker's whole state while it is up, and nil the rest of the time. `typing` is nil
-- except while a name is being entered, when it is the string so far.
local state = nil
local win = nil
local mode_handle = nil

-- Where the panes are, in surface coordinates, as of the last tick. The picker is centred over
-- them rather than over the terminal, so it does not sit on the status bar or the sidebar —
-- and it is remembered rather than asked for, because a key is pressed between two ticks and
-- `ctx` belongs to a tick.
local area = nil

-- A name a session may have, one character at a time: the host refuses anything else, so
-- refusing it here is the difference between a key that does nothing and an op that is logged
-- as a mistake.
local function nameable(key)
  return #key == 1 and key:match("[A-Za-z0-9_-]") ~= nil
end

-- What a session is doing: how many panes it has, and what the one somebody is looking at is
-- running. Read through `with`, which is what lets a picker say anything at all about a
-- session nobody is attached to.
local function doing(name)
  local panes, command, worst = 0, nil, "idle"
  neotmux.session.with(name, function()
    local list = neotmux.pane.list()
    panes = #list
    local focus = neotmux.pane.current()
    local info = focus and neotmux.pane.info(focus)
    command = info and info.command
    worst = pane_state.worst(list)
  end)
  return panes, command, worst
end

local function leave()
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

-- The rows the window shows, taken fresh: a session can appear or go while the picker is up,
-- and the cursor is clamped to whatever is there now rather than to what was there on the way
-- in.
local function refresh()
  state.names = neotmux.session.list()
  if #state.names == 0 then
    state.at = 1
  elseif state.at > #state.names then
    state.at = #state.names
  elseif state.at < 1 then
    state.at = 1
  end
end

M.commands = {
  ["down"] = function()
    state.at = state.at + 1
    if state.at > #state.names then
      state.at = 1
    end
  end,
  ["up"] = function()
    state.at = state.at - 1
    if state.at < 1 then
      state.at = #state.names
    end
  end,
  ["choose"] = function()
    local name = state.names[state.at]
    if name and name ~= neotmux.session.current() then
      neotmux.session.switch(name)
    end
    leave()
  end,
  ["create"] = function()
    state.typing = ""
  end,
  ["kill"] = function()
    local name = state.names[state.at]
    -- The one you are in is refused rather than ended: ending it would leave this client with
    -- nothing to be attached to, and the picker with nothing to close over.
    if name and name ~= neotmux.session.current() then
      neotmux.session.kill(name)
      table.remove(state.names, state.at)
      if state.at > #state.names then
        state.at = math.max(#state.names, 1)
      end
    end
  end,
  ["cancel"] = leave,
}

-- The name field's own keys, which are every printable character and three that are not.
local function typing(key)
  if key == "Esc" then
    state.typing = nil
  elseif key == "Enter" then
    local name = state.typing
    state.typing = nil
    if name ~= "" then
      -- Two ops in one key, applied in the order they were raised: the session exists by the
      -- time the switch is applied, which is the frame after this one.
      neotmux.session.create(name)
      neotmux.session.switch(name)
      leave()
    end
  elseif key == "BS" or key == "Backspace" then
    state.typing = state.typing:sub(1, -2)
  elseif nameable(key) then
    state.typing = state.typing .. key
  end
end

-- How big the window wants to be and where: the list, the key line, and the frame's own two
-- rows, centred over the panes.
local function wanted()
  local rows = math.min(#state.names, M.rows) + 3
  local w = math.min(M.width, area and area.w or M.width)
  local h = math.min(rows, area and area.h or rows)
  local x = (area and area.x or 0) + math.max(0, math.floor(((area and area.w or w) - w) / 2))
  local y = (area and area.y or 0) + math.max(0, math.floor(((area and area.h or h) - h) / 2))
  return { x = x, y = y, w = w, h = h }
end

function M.open()
  if state then
    leave()
    return
  end
  state = { at = 1, names = neotmux.session.list(), typing = nil }
  -- Over the middle of what you were looking at. `float` is refused rather than clipped when
  -- the terminal cannot hold it, and a refusal here is a picker that does not open.
  local handle = neotmux.float(wanted())
  if not handle then
    state = nil
    return
  end
  win = handle
  mode_handle = modes.raise("sessions")
end

if package.loaded["plugin.prefix"] then
  require("plugin.prefix").commands["s"] = function()
    M.open()
  end
end

neotmux.on_key(function(key)
  if state == nil then
    return false
  end
  if state.typing then
    typing(key)
  else
    refresh()
    local command = M.commands[M.keys[key]]
    if command then
      command()
    end
  end
  return true
end)

neotmux.on_tick(function(ctx)
  -- Remembered every tick, picker or no: it is what the next open is placed against.
  local left, top, right, bottom
  for _, pane in ipairs(neotmux.pane.list()) do
    local r = ctx:pane_rect(pane)
    if r then
      left = math.min(left or r.x, r.x)
      top = math.min(top or r.y, r.y)
      right = math.max(right or (r.x + r.w), r.x + r.w)
      bottom = math.max(bottom or (r.y + r.h), r.y + r.h)
    end
  end
  if left then
    area = { x = left, y = top, w = right - left, h = bottom - top }
  end

  if state == nil or win == nil then
    return
  end
  refresh()
  local rect = ctx:float_rect(win)
  if rect == nil then
    return
  end
  -- The list grows and shrinks while the picker is up, and the terminal can be resized under
  -- it. Both are one comparison and two calls that do nothing when nothing moved.
  local want = wanted()
  if rect.w ~= want.w or rect.h ~= want.h then
    win:resize_to(want.w, want.h)
  end
  if rect.x ~= want.x or rect.y ~= want.y then
    win:move_to(want.x, want.y)
  end
  local inner = box.draw(ctx, win, rect, { title = "sessions", bg = M.colors.frame })

  local here = neotmux.session.current()
  local room = inner.h - 1
  for at = 1, math.min(#state.names, room) do
    local name = state.names[at]
    local row = inner.y + at - 1
    local chosen = at == state.at
    if chosen then
      ctx:rect(win, inner.x, row, inner.w, 1, { bg = M.colors.cursor })
    end
    local background = chosen and M.colors.cursor or M.colors.frame
    local panes, command, worst = doing(name)
    local said = (command or "-") .. "  " .. panes .. "p"
    ctx:text(win, inner.x, row, pane_state.icon(worst), {
      fg = M.colors[worst],
      bg = background,
    })
    -- The name on the left and what it is running on the right, so the two read as columns
    -- rather than as one run of words. A name too long for the row keeps the row: the right
    -- column is dropped instead, because it is the one you can do without.
    -- The icon owns the first column and the names start after it, so a list of them reads as
    -- a column rather than as a run of words with punctuation in front.
    local label = inner.x + 2
    ctx:text(win, label, row, name, {
      fg = name == here and M.colors.current or M.colors.name,
      bg = background,
    })
    local at_col = inner.x + inner.w - neotmux.width(said) - 1
    if at_col > label + neotmux.width(name) + 1 then
      ctx:text(win, at_col, row, said, { fg = M.colors.doing, bg = background })
    end
  end

  local last = inner.y + inner.h - 1
  if state.typing then
    local label = "name  "
    ctx:text(win, inner.x + 2, last, label .. state.typing, {
      fg = M.colors.name,
      bg = M.colors.frame,
    })
    -- The caret, drawn rather than placed: the terminal's own cursor belongs to the focused
    -- pane, and a float cannot take it.
    ctx:rect(
      win,
      inner.x + 2 + neotmux.width(label) + neotmux.width(state.typing),
      last,
      1,
      1,
      { bg = M.colors.caret }
    )
  else
    ctx:text(win, inner.x + 2, last, "j k  enter  c new  d kill  esc", {
      fg = M.colors.hint,
      bg = M.colors.frame,
    })
  end
end)

return M

-- When every line of every pane arrived, and how long each one kept you waiting.
--
-- A terminal forgets the one thing about a line it cannot get back: when it came. This keeps
-- it, for every pane in every window, and `C-b t` (or the palette's `timestamps`) shows it in a
-- band down each pane's left edge — the time a line arrived, where it differs from the line
-- above, yellow or red when the pane sat still before it:
--
--   require('plugin.stamps').stall = { slow = 2, stuck = 10 }   -- seconds: yellow, then red
--
-- The band is the pane's own columns, not drawn over them: while it is shown, every pane's
-- shell is that many columns narrower, and hiding it gives them back. A time is known per tick,
-- not per line — lines that came together get the same one — and only a line that is new gets
-- one: a progress bar rewriting its row keeps the time it first appeared, and nothing is kept
-- while a pane is on its alternate screen. A line that falls off the pane's history takes its
-- time with it.
local g = neotmux.group("plugin.stamps")
local gutter = require("neotmux.gutter")

local M = {
  shown = false,
  -- How long a pane sat before a line for the band to say so, in seconds.
  stall = { slow = 2, stuck = 10 },
  -- `HH:MM:SS`, a space, and the rule.
  width = 10,
  colors = {
    time = "#6c7086",
    rule = "#45475a",
    slow = "#f9e2af",
    stuck = "#f38ba8",
  },
}

-- Per pane: `next`, the line number the pane will write next, and `at`, each line's arrival
-- in `os.time()` seconds, keyed by line number. A line number counts every line the pane has
-- ever had — those fallen off its history, those in it, then the screen's rows — so it names
-- the same line however far the screen has scrolled since.
local panes = {}

-- The line number of the screen's top row, and the one the cursor is on.
local function numbers(pane)
  local len, dropped = neotmux.pane.history(pane)
  local cursor = neotmux.pane.cursor(pane)
  if len == nil or cursor == nil then
    return nil
  end
  local top = dropped + len
  return top, top + cursor.y, dropped
end

local function record(pane, now)
  local mode = neotmux.pane.mode(pane)
  local top, cursor, dropped = numbers(pane)
  if top == nil then
    return
  end
  local it = panes[pane]
  if it == nil or (mode and mode.alternate) then
    -- A new pane starts from where it is, and a full-screen program's rows are not lines.
    panes[pane] = { next = cursor, at = (it and it.at) or {} }
    return
  end
  if cursor > it.next then
    for line = it.next, cursor - 1 do
      it.at[line] = it.at[line] or now
    end
    it.oldest = it.oldest or it.next
  elseif cursor < it.next then
    -- The screen cleared, or the cursor sent back up: what is written from here on is new,
    -- whatever line number it lands on.
    for line = cursor, it.next - 1 do
      it.at[line] = nil
    end
  end
  it.next = cursor
  -- The lines the history has let go of, oldest first, and no further than that.
  while it.oldest and it.oldest < dropped do
    it.at[it.oldest] = nil
    it.oldest = it.oldest + 1
  end
  it.top = top
end

local function draw(ctx, pane)
  local it = panes[pane]
  local rect = ctx:pane_band(pane)
  if it == nil or rect == nil or it.top == nil then
    return
  end
  -- Wound back with the pane when copy mode, or anything else, draws it into its history.
  local top = it.top - neotmux.pane.view(pane)
  gutter.rule(ctx, rect, M.colors.rule)
  for row = 0, rect.h - 1 do
    local line = top + row
    local at = it.at[line]
    local before = it.at[line - 1]
    if at and before ~= at then
      local waited = before and at - before or 0
      local fg = M.colors.time
      if waited >= M.stall.stuck then
        fg = M.colors.stuck
      elseif waited >= M.stall.slow then
        fg = M.colors.slow
      end
      ctx:text(
        nil,
        rect.x,
        rect.y + row,
        os.date("%H:%M:%S", at):sub(1, math.max(0, rect.w - 2)),
        { fg = fg }
      )
    end
  end
end

g.on_tick(function(ctx)
  local now = os.time()
  for _, pane in ipairs(gutter.every_pane()) do
    record(pane, now)
  end
  if M.shown then
    for _, pane in ipairs(neotmux.pane.list()) do
      draw(ctx, pane)
    end
  end
end)

g.on_event("pane_closed", function(event)
  panes[event.pane] = nil
end)

M.toggle = gutter.toggler(M, "plugin.stamps", function(width)
  return g.reserve_pane("left", width)
end)

g.command.add("timestamps", M.toggle, {
  desc = "show when each line of every pane arrived, and how long it waited",
})

require("neotmux.bind").prefix("t", "timestamps")

return M

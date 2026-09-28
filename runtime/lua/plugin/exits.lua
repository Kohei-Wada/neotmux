-- How each command in every pane ended, and how long it took.
--
-- `C-b e` (or the palette's `exits`) shows it in a band down each pane's left edge, on the line
-- each command was typed on: `✔` for a zero exit, `✘` and the status otherwise, `?` for an
-- end that carried no status, `⋯` while it runs, then how long it took or has been running.
--
-- It reads `neotmux.commands`, so it needs a shell that sends OSC 133 marks — the lines to add
-- to bash or zsh are in docs/plugins.md. A pane whose shell sends none has an empty band, and
-- so does one with a full-screen program up. A command whose end nobody saw (`unknown`) and
-- the prompt being typed at are left blank.
--
-- The time is when this plugin first saw the command's output start and its end, to the tick,
-- so a command that had already finished when it first looked has no time.
local g = neotmux.group("plugin.exits")
local commands = require("neotmux.commands")

local M = {
  shown = false,
  -- The mark, the status in three, a space, the time in three, and the rule.
  width = 9,
  colors = {
    ok = "#a6e3a1",
    failed = "#f38ba8",
    running = "#f9e2af",
    time = "#6c7086",
    rule = "#45475a",
  },
}

local band

-- Per pane: `at`, when each `output` and `done` mark was first seen, in `neotmux.now()`
-- milliseconds, as `at[line][kind]`; `false` for a mark whose time nobody knows, because it was
-- already on the screen the first time the pane was looked at. `dropped` is how many lines had
-- fallen off the history then, and `last` the lowest line anything is kept for.
--
-- Only the screen is read, since that is where marks arrive and where a redrawn prompt writes
-- over old ones. A line in the history cannot be written again, so what is known about its
-- marks is kept until the history drops it: copy mode winds back to it later, and a command
-- watched on the screen still says how long it took. A mark that went into the history before
-- it was ever read has no time.
local seen = {}

-- The pane's live top row and the row drawn at its top, which is the live one wound back by
-- whatever view is up, or nil for a full-screen program, whose rows are not the shell's lines
-- and whose screen keeps no history to count from.
local function tops(pane)
  local mode = neotmux.pane.mode(pane)
  if mode and mode.alternate then
    return nil
  end
  local len, dropped = neotmux.pane.history(pane)
  if len == nil then
    return nil
  end
  local live = dropped + len
  return live, live - neotmux.pane.view(pane), dropped
end

local function note(pane, now)
  local live, _, dropped = tops(pane)
  if live == nil then
    return
  end
  local it = seen[pane]
  local first = it == nil
  it = it or { at = {}, dropped = dropped, last = live }
  local present = {}
  for _, mark in ipairs(neotmux.pane.marks(pane, live) or {}) do
    if mark.kind == "output" or mark.kind == "done" then
      local line = it.at[mark.line] or {}
      it.at[mark.line] = line
      if line[mark.kind] == nil then
        line[mark.kind] = not first and now or false
      end
      present[mark.line] = present[mark.line] or {}
      present[mark.line][mark.kind] = true
      it.last = math.max(it.last, mark.line)
    end
  end
  -- A mark on the screen that is no longer there was written over: `clear`, or a redrawn prompt.
  for line = live, it.last do
    local kinds = it.at[line]
    if kinds then
      for kind in pairs(kinds) do
        if not (present[line] and present[line][kind]) then
          kinds[kind] = nil
        end
      end
      if next(kinds) == nil then
        it.at[line] = nil
      end
    end
  end
  for line = it.dropped, dropped - 1 do
    it.at[line] = nil
  end
  it.dropped = dropped
  seen[pane] = it
end

local function when(pane, line, kind)
  local it = seen[pane]
  local kinds = it and line and it.at[line]
  return kinds and kinds[kind] or nil
end

local function span(ms)
  local s = math.floor(ms / 1000)
  if s < 60 then
    return s .. "s"
  elseif s < 3600 then
    return math.floor(s / 60) .. "m"
  elseif s < 86400 then
    return math.floor(s / 3600) .. "h"
  end
  return math.floor(s / 86400) .. "d"
end

local function draw(ctx, pane, now)
  local rect = ctx:pane_band(pane)
  if rect == nil then
    return
  end
  local rule = rect.x + rect.w - 1
  for row = 0, rect.h - 1 do
    ctx:text(nil, rule, rect.y + row, "│", { fg = M.colors.rule })
  end
  local _, top = tops(pane)
  if top == nil then
    return
  end
  for _, command in ipairs(commands.of(pane, top, top + rect.h - 1)) do
    local row = (command.input or command.prompt) - top
    if row >= 0 and row < rect.h and (command.state == "done" or command.state == "running") then
      local y = rect.y + row
      local started = when(pane, command.output, "output")
      local ended
      if command.state == "running" then
        ctx:text(nil, rect.x, y, "⋯", { fg = M.colors.running })
        ended = now
      elseif command.exit == nil then
        ctx:text(nil, rect.x, y, "?", { fg = M.colors.time })
        ended = when(pane, command.done, "done")
      else
        local failed = command.exit ~= 0
        local colour = failed and M.colors.failed or M.colors.ok
        ctx:text(nil, rect.x, y, failed and "✘" or "✔", { fg = colour })
        if failed and rect.w >= 5 then
          ctx:text(nil, rect.x + 1, y, string.format("%3s", tostring(command.exit):sub(-3)), {
            fg = colour,
          })
        end
        ended = when(pane, command.done, "done")
      end
      if started and ended and rect.w >= 9 then
        ctx:text(nil, rect.x + 5, y, string.format("%3s", span(ended - started)), {
          fg = M.colors.time,
        })
      end
    end
  end
end

-- Every window's panes, the hidden ones too: the build in a window you are not looking at is
-- the one whose time you will want when you come back to it.
local function every_pane()
  local out, listed = {}, {}
  local function add(pane)
    if not listed[pane] then
      listed[pane] = true
      out[#out + 1] = pane
    end
  end
  for _, window in ipairs(neotmux.window.list()) do
    for _, pane in ipairs(neotmux.window.panes(window) or {}) do
      add(pane)
    end
  end
  for _, pane in ipairs(neotmux.pane.list()) do
    add(pane)
  end
  return out
end

g.on_tick(function(ctx)
  local now = neotmux.now()
  for _, pane in ipairs(every_pane()) do
    note(pane, now)
  end
  if M.shown then
    for _, pane in ipairs(neotmux.pane.list()) do
      draw(ctx, pane, now)
    end
  end
end)

g.on_event("pane_closed", function(event)
  seen[event.pane] = nil
end)

function M.toggle()
  M.shown = not M.shown
  if not M.shown then
    if band then
      band:remove()
      band = nil
    end
    return
  end
  if M.width < 1 or M.width > 24 then
    M.shown = false
    neotmux.notify("warn", "plugin.exits: width must be 1..24, got " .. tostring(M.width))
    return
  end
  local handle, message = g.reserve_pane("left", M.width)
  if handle == nil then
    M.shown = false
    neotmux.notify("warn", "plugin.exits: " .. message)
    return
  end
  band = handle
end

g.command.add("exits", M.toggle, {
  desc = "show how each command in every pane ended and how long it took",
})

if package.loaded["plugin.tmux.prefix"] then
  require("plugin.tmux.prefix").commands["e"] = "exits"
end

return M

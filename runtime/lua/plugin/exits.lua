-- How each command in every pane ended, and how long it took.
--
-- `C-b e` (or the palette's `exits`) shows it in a band down each pane's left edge, on the line
-- each command was typed on: `✔` for a zero exit, `✘` and the status otherwise, `⋯` while it
-- runs, then how long it took or has been running.
--
-- It reads `neotmux.commands`, so it needs a shell that sends OSC 133 marks — the lines to add
-- to bash or zsh are in docs/plugins.md. A pane whose shell sends none has an empty band. A
-- command whose end nobody saw (`unknown`) and the prompt being typed at are left blank.
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

-- Per pane, when each of its marks was first seen, in `neotmux.now()` milliseconds, keyed by
-- line and kind; `false` for the marks already there the first time the pane was looked at.
local seen = {}

local function note(pane, now)
  local marks = neotmux.pane.marks(pane)
  if marks == nil then
    return
  end
  local was = seen[pane]
  local now_seen = {}
  for _, mark in ipairs(marks) do
    local key = mark.line .. ":" .. mark.kind
    if was == nil then
      now_seen[key] = false
    elseif was[key] ~= nil then
      now_seen[key] = was[key]
    else
      now_seen[key] = now
    end
  end
  seen[pane] = now_seen
end

local function when(pane, line, kind)
  local at = seen[pane] and line and seen[pane][line .. ":" .. kind]
  return at or nil
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
  local len, dropped = neotmux.pane.history(pane)
  if len == nil then
    return
  end
  local top = dropped + len
  for _, command in ipairs(commands.of(pane)) do
    local row = command.prompt - top
    if row >= 0 and row < rect.h and (command.state == "done" or command.state == "running") then
      local y = rect.y + row
      local started = when(pane, command.output, "output")
      local ended
      if command.state == "done" then
        local failed = command.exit ~= nil and command.exit ~= 0
        local colour = failed and M.colors.failed or M.colors.ok
        ctx:text(nil, rect.x, y, failed and "✘" or "✔", { fg = colour })
        if failed then
          ctx:text(nil, rect.x + 1, y, string.format("%3s", tostring(command.exit):sub(-3)), {
            fg = colour,
          })
        end
        ended = when(pane, command.done, "done")
      else
        ctx:text(nil, rect.x, y, "⋯", { fg = M.colors.running })
        ended = now
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
  if M.width < 1 or M.width > 16 then
    M.shown = false
    neotmux.notify("warn", "plugin.exits: width must be 1..16, got " .. tostring(M.width))
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

-- Copy mode: read back over what a pane has already said, select a region of it, and take
-- the text.
--
-- On by default, with the status bar and the prefix rather than with the watcher and the
-- sidebar. It spends no screen space — no band, no gutter, no column off any pane — and it
-- costs one key out of the prefix's table. What it is for is not an opinion about what you
-- run either: a pane keeps ten thousand lines the host will never show you unless something
-- asks for them, and a multiplexer with no way to look at them is missing the use of its own
-- scrollback. Turn it off the way any default goes off:
--
--   require('neotmux.defaults').copy = false
--
--   C-b [       enter, on the pane you are in
--   h j k l     the cursor, one cell
--   C-u C-d     half a screen
--   g G         the oldest line the snapshot holds, the newest
--   Space       start a selection here, or drop the one there is
--   y           yank it
--   Esc q       leave
--
-- Nothing here is a host feature. The mode is an `on_key` over a local, exactly as the
-- prefix's own two are; the lines are `pane.scrollback` and `pane.content`, taken once;
-- `ctx:tint` is the selection, drawn over the pane's real text rather than in place of it.
--
-- The three pieces are three files on purpose. `neotmux.snapshot` is the buffer and knows
-- nothing about keys; `neotmux.selection` is arithmetic over two points and has never seen a
-- pane; this file is the mode. Changing what a key does touches one table in one of them.
local snapshot = require("neotmux.snapshot")
local selection = require("neotmux.selection")

local M = {}

-- Read every tick, so assigning to one from your own `init.lua` takes effect.
M.colors = {
  -- Catppuccin Mocha surface1: a background dark enough that the pane's own text is still
  -- readable through it, which is the whole reason the selection is a tint and not a rect.
  selection = "#45475a",
  cursor = "#f9e2af", -- yellow
  -- The cursor names both channels. A tint that changed only the background would leave a
  -- cell whose text happened to be yellow invisible, and the cursor is the one cell that must
  -- never be hard to find.
  cursor_text = "#1e1e2e",
}

-- How far a half-page key goes, as a fraction of the pane's height.
M.page = 0.5

-- Where a yank puts the text.
--
-- Not the system clipboard by itself: this is the buffer, which always gets the text, and
-- reaching the terminal's clipboard is a separate thing that may or may not be possible (see
-- `clipboard` below). Keeping the two apart is what lets a paste command attach here later
-- without caring whether the clipboard worked.
--
--   neotmux.log.info(require('plugin.copy').register)
M.register = ""

-- The mode, and everything in it. Nil when it is off, which is also what makes the tick below
-- free while nobody is copying.
local state = nil

local function clamp(value, low, high)
  return math.max(low, math.min(high, value))
end

-- The cursor back inside the buffer, and the view scrolled just far enough to hold it. Just
-- far enough and no further: `j` at the bottom edge moves the view one line, which is what
-- reading is, rather than half a screen.
local function settle(rect)
  local cursor = state.cursor
  cursor.line = clamp(cursor.line, 0, state.buffer.total - 1)
  cursor.col = clamp(cursor.col, 0, rect.w - 1)
  state.top = clamp(state.top, 0, math.max(0, state.buffer.total - rect.h))
  if cursor.line < state.top then
    state.top = cursor.line
  elseif cursor.line > state.top + rect.h - 1 then
    state.top = cursor.line - rect.h + 1
  end
end

local function page(rect)
  return math.max(1, math.floor(rect.h * M.page))
end

-- The terminal's own clipboard, if this host can reach one at all.
--
-- Written against a call that is not in the ABI today. `neotmux` answers nil for a field it
-- does not have, so the probe is safe now and becomes the real thing the moment the call
-- lands. The `== true` is the important half: a clipboard that did not say it worked did not
-- work. tmux returns silently when terminfo has no `Ms`, and a copy mode that reports success
-- on that is telling you your text is somewhere it is not.
local function clipboard(text)
  local set = neotmux.clipboard and neotmux.clipboard.set
  return set ~= nil and set(text) == true
end

-- Everything a key can do, by name. The table is the point: `on_key` below looks a key up in
-- `M.keys` and a name up in here, and does nothing else. Rebinding is then one assignment
-- from an `init.lua`, and a command is nameable from outside this file:
--
--   local copy = require('plugin.copy')
--   copy.keys['v'] = 'begin-selection'
--   copy.keys['C-b'] = nil
--
-- Each one takes the mode's own table and the pane's rect. The buffer is a snapshot, so the
-- number of lines in it never moves under a command.
M.commands = {
  ["cursor-left"] = function(mode)
    mode.cursor.col = mode.cursor.col - 1
  end,
  ["cursor-right"] = function(mode)
    mode.cursor.col = mode.cursor.col + 1
  end,
  ["cursor-up"] = function(mode)
    mode.cursor.line = mode.cursor.line - 1
  end,
  ["cursor-down"] = function(mode)
    mode.cursor.line = mode.cursor.line + 1
  end,
  ["halfpage-up"] = function(mode, rect)
    mode.cursor.line = mode.cursor.line - page(rect)
  end,
  ["halfpage-down"] = function(mode, rect)
    mode.cursor.line = mode.cursor.line + page(rect)
  end,
  ["history-top"] = function(mode)
    mode.cursor.line = 0
  end,
  ["history-bottom"] = function(mode)
    mode.cursor.line = mode.buffer.total - 1
  end,
  ["begin-selection"] = function(mode)
    mode.anchor = mode.anchor == nil and { line = mode.cursor.line, col = mode.cursor.col }
      or nil
  end,
  ["copy-selection"] = function(mode)
    if mode.anchor == nil then
      return
    end
    local first, last = selection.range(mode.anchor, mode.cursor)
    -- The buffer is fixed, so this reads the same lines the cursor was dragged over rather
    -- than whatever the pane has printed since.
    M.register = selection.text(first, last, function(line)
      return snapshot.line(mode.buffer, line)
    end)
    mode.anchor = nil
    local count = last.line - first.line + 1
    local where = clipboard(M.register) and "the system clipboard" or "plugin.copy.register"
    neotmux.notify(
      "info",
      "copied " .. count .. (count == 1 and " line" or " lines") .. " into " .. where
    )
  end,
  ["cancel"] = function()
    state = nil
  end,
}

-- Which key runs which command. A key with no entry does nothing and is still swallowed:
-- every key in the mode belongs to the mode, and dropping a half-made selection over a typo
-- is not a trade worth making when `Esc` and `q` are both already the way out.
M.keys = {
  ["h"] = "cursor-left",
  ["l"] = "cursor-right",
  ["k"] = "cursor-up",
  ["j"] = "cursor-down",
  ["C-u"] = "halfpage-up",
  ["C-d"] = "halfpage-down",
  ["g"] = "history-top",
  ["G"] = "history-bottom",
  [" "] = "begin-selection",
  ["y"] = "copy-selection",
  ["Esc"] = "cancel",
  ["q"] = "cancel",
}

-- The mode opens where the pane already is: the view is the visible screen and the cursor is
-- its top left corner, so entering changes nothing about what is on screen.
function M.enter()
  local pane = neotmux.pane.current()
  if pane == nil then
    return
  end
  local buffer = snapshot.take(pane)
  if buffer == nil then
    return
  end
  state = {
    pane = pane,
    buffer = buffer,
    top = buffer.history,
    cursor = { line = buffer.history, col = 0 },
    anchor = nil,
  }
end

neotmux.on_key(function(key)
  if state == nil then
    return false
  end
  local rect = neotmux.pane.rect(state.pane)
  if rect == nil then
    -- The pane went out from under the mode. Leave, and hand the key on rather than eating it.
    state = nil
    return false
  end
  local command = M.commands[M.keys[key]]
  if command then
    command(state, rect)
  end
  if state then
    settle(rect)
  end
  return true
end)

-- The mode belongs to the pane it was entered on. Both of these would otherwise leave it
-- drawing a cursor over a pane nobody is looking at.
neotmux.on_event("pane_closed", function(event)
  if state and state.pane == event.pane then
    state = nil
  end
end)

neotmux.on_event("focus_changed", function(event)
  if state and state.pane ~= event.pane then
    state = nil
  end
end)

neotmux.on_tick(function(ctx)
  if state == nil then
    return
  end
  local rect = ctx:pane_rect(state.pane)
  if rect == nil then
    return
  end
  settle(rect)

  -- What the view costs, and why the mode is built this way.
  --
  -- The pane's own grid is what the host renders. There is exactly one case where that grid
  -- already shows what the mode wants shown — the view sitting on the snapshot's own screen,
  -- over a pane that has not been fed since the snapshot was taken — and in that case the
  -- frame is one `ctx:tint` for the cursor and nothing else. Any other case, scrolled or
  -- overtaken, and every row is wrong: the pane is erased and redrawn out of the snapshot,
  -- one `ctx:rect` and one `ctx:text` per non-blank row, which on a full-width pane of 23
  -- rows is 25 commands.
  --
  -- No read is on that bill at all. The lines were taken once, on the way in; the tick slices
  -- a table. That is the difference between this and a mode that asks the host for its screen
  -- every frame, and it is the same arrangement that makes the text coherent.
  local showing = state.top == state.buffer.history
    and neotmux.pane.revision(state.pane) == state.buffer.revision
  if not showing then
    ctx:rect(state.pane, 0, 0, rect.w, rect.h)
    for at, line in ipairs(snapshot.window(state.buffer, state.top, rect.h)) do
      if line ~= "" then
        -- Text, because text is what the ABI hands back: a line out of the snapshot has no
        -- colours on it, so a redrawn view is monochrome and the tint below is the only
        -- colour in it.
        ctx:text(state.pane, 0, at - 1, line)
      end
    end
  end

  if state.anchor then
    -- The selection is two absolute points; this is where they are projected onto the rows
    -- that happen to be on screen. An end scrolled out of view is clamped by `span` rather
    -- than lost, and a view that has left the selection behind entirely draws nothing —
    -- hidden, not deleted.
    for row = 0, rect.h - 1 do
      local x, width = selection.span(state.anchor, state.cursor, state.top + row, rect.w)
      if x then
        -- Tint and not rect: the selection has to be visible over the pane's real content,
        -- and rect would paint the text out of it.
        ctx:tint(state.pane, x, row, width, 1, { bg = M.colors.selection })
      end
    end
  end

  ctx:tint(state.pane, state.cursor.col, state.cursor.line - state.top, 1, 1, {
    fg = M.colors.cursor_text,
    bg = M.colors.cursor,
  })
end)

-- The prefix is the standard plugin and this one hangs off it, so this is the direction the
-- knowledge goes in — the same way the sidebar adds its own fold key. `[` is where tmux puts
-- it.
--
-- The defaults table and not just `package.loaded`, because a user who rebinds one of the
-- keys above has to `require` this file from their own `init.lua`, which runs before the
-- shipped layer's — so the prefix is not loaded yet and a plain probe would silently leave
-- `C-b [` unbound. Loading it here is what it was about to do anyway.
if package.loaded["plugin.prefix"] or require("neotmux.defaults").prefix then
  require("plugin.prefix").commands["["] = M.enter
end

return M

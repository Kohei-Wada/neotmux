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
--   w b e       a word forward, a word back, the end of one
--   0 ^ $       the first column, the first word, the last
--   C-u C-d     half a screen
--   g G         the oldest line the snapshot holds, the newest
--   v           start a selection here, or drop the one there is
--   y           yank it, and leave
--   f           label every word on screen, jump the cursor to the one you pick
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
local words = require("neotmux.words")
local modes = require("neotmux.mode")
local hints = require("neotmux.hints")

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
-- The handle `neotmux.mode.raise` returns while `state` is up, and nil the rest of the time.
-- Kept apart from `state` rather than folded into it because every exit — cancel, the pane
-- closing, focus moving away — has to lower it, and `leave` below is the one place that does.
local mode_handle = nil
-- The handle `hints.open` returns while a `f` pick is in progress, and nil the rest of the
-- time. It is what closes a pick this mode started when the mode itself is leaving; which keys
-- go somewhere else is a question the mode stack answers instead, since whatever is raised
-- over this mode is something we must not be typing into, ours or not.
local hint_handle = nil

local function leave()
  if hint_handle then
    -- Closed here too, not only by its own Esc: `pane_closed` and `focus_changed` reach
    -- `leave` straight from an event, never through this mode's own `on_key`, so a pick left
    -- pending would otherwise still be open, drawing over the pane the cursor has left.
    hint_handle:close()
    hint_handle = nil
  end
  if state and state.froze then
    -- The pane goes back to showing what it is doing, and to moving while it does it — unless
    -- it was already being held still when the mode was entered, in which case whoever froze
    -- it is still using it and this is not the mode that thaws it.
    neotmux.pane.unfreeze(state.pane)
  end
  if mode_handle then
    mode_handle:lower()
    mode_handle = nil
  end
  state = nil
end

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

-- The text between two columns, inclusive, out of a line — columns and not bytes, the same
-- reason `neotmux.hints` counts that way: a run found by column has to be sliced by column
-- too, or a wide character in front of it shifts every answer after it.
local function slice(line, from_col, to_col)
  local out = {}
  local col = 0
  for _, code in utf8.codes(line) do
    local ch = utf8.char(code)
    if col >= from_col and col <= to_col then
      out[#out + 1] = ch
    end
    col = col + math.max(1, neotmux.width(ch))
    if col > to_col then
      break
    end
  end
  return table.concat(out)
end

-- The column the run starting at `col` ends at — its own last column, not the next run's
-- first one. `words.word_end` cannot be reused for this: it is built for `e`, and vim's `e`
-- from a word already at its own end lands on the *next* word's end, which is correct for a
-- motion and wrong for a boundary. This walks the same word/punct/blank classes `neotmux.words`
-- classifies with, from `col`, stopping the instant the class changes — blank included — so a
-- run's `finish` never reaches past its own last character into the blanks after it.
local function run_end(line, col)
  local class = nil
  local finish = col
  local at = 0
  for _, code in utf8.codes(line) do
    local ch = utf8.char(code)
    if at >= col then
      local this = (ch ~= " " and ch ~= "\t") and (ch:match(words.word) and "word" or "punct")
        or "blank"
      if class == nil then
        class = this
      elseif this ~= class then
        break
      end
      finish = at
    end
    at = at + math.max(1, neotmux.width(ch))
  end
  return finish
end

-- Every word on one line, `neotmux.words`' own idea of one: a run `w` would step over, not
-- the url/path/hash/ip set `neotmux.hints` reads by default. In copy mode the thing you want
-- is a position to land the cursor on, not a token to act on, so this is the finder for it.
local function line_words(line)
  -- `first_word` answers 0 both for "the first word is at column 0" and for "there is no
  -- word on this line at all" — it is built for `^`, which always needs a column to land the
  -- cursor on. Checked here, not there: a line with nothing on it gets no target.
  if not line:match("%S") then
    return {}
  end
  local out = {}
  local col = words.first_word(line)
  while col do
    local finish = run_end(line, col)
    out[#out + 1] = { col = col, text = slice(line, col, finish) }
    local next_col = words.forward(line, col)
    if next_col == nil or next_col <= col then
      break
    end
    col = next_col
  end
  return out
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
  -- The word motions, which are about the line the cursor is on. `w` and `b` step to the
  -- next line where there is nothing left on this one, the way vim does; `e`, `0`, `^` and
  -- `$` stay where they are.
  ["word-forward"] = function(mode)
    local line = snapshot.line(mode.buffer, mode.cursor.line) or ""
    local col = words.forward(line, mode.cursor.col)
    if col then
      mode.cursor.col = col
    else
      mode.cursor.line = mode.cursor.line + 1
      mode.cursor.col = words.first_word(snapshot.line(mode.buffer, mode.cursor.line) or "")
    end
  end,
  ["word-back"] = function(mode)
    local line = snapshot.line(mode.buffer, mode.cursor.line) or ""
    local col = words.back(line, mode.cursor.col)
    if col then
      mode.cursor.col = col
    elseif mode.cursor.line > 0 then
      mode.cursor.line = mode.cursor.line - 1
      mode.cursor.col = words.last(snapshot.line(mode.buffer, mode.cursor.line) or "")
    end
  end,
  ["word-end"] = function(mode)
    local line = snapshot.line(mode.buffer, mode.cursor.line) or ""
    local col = words.word_end(line, mode.cursor.col)
    if col then
      mode.cursor.col = col
    end
  end,
  ["line-start"] = function(mode)
    mode.cursor.col = 0
  end,
  ["line-first-word"] = function(mode)
    mode.cursor.col = words.first_word(snapshot.line(mode.buffer, mode.cursor.line) or "")
  end,
  ["line-end"] = function(mode)
    mode.cursor.col = words.last(snapshot.line(mode.buffer, mode.cursor.line) or "")
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
    end, neotmux.width)
    mode.anchor = nil
    local count = last.line - first.line + 1
    -- The register is a fact; the clipboard is not. `clipboard.set` returning true says the
    -- escape reached the terminal, not that the terminal took it — OSC 52 has no reply, and a
    -- terminal that ignores it is byte-identical to one that accepted it. Saying "copied to
    -- the clipboard" on the strength of that is the false-success bug other multiplexers ship.
    local offered = clipboard(M.register)
    -- Taken, so there is nothing left to do here: the yank is what the mode was entered for,
    -- and tmux leaves on it too. The notify below says what was taken, which is the only
    -- reason to still be looking at the pane.
    leave()
    neotmux.notify(
      "info",
      "yanked "
        .. count
        .. (count == 1 and " line" or " lines")
        .. " into plugin.copy.register"
        .. (offered and ", and offered them to the terminal's clipboard" or "")
    )
  end,
  -- Every word on the screen the mode is showing, labelled; the label answers with a
  -- screen-relative row and column, same as `neotmux.hints`' own default finders do, so it is
  -- `mode.top` that turns it back into a row the buffer recognises — the view is scrolled,
  -- the buffer is not.
  ["hint-jump"] = function(mode, rect)
    hint_handle = hints.open({
      panes = { mode.pane },
      find = function()
        local out = {}
        for row = 0, rect.h - 1 do
          local line = snapshot.line(mode.buffer, mode.top + row)
          if line then
            for _, hit in ipairs(line_words(line)) do
              out[#out + 1] = { line = row, col = hit.col, text = hit.text }
            end
          end
        end
        return out
      end,
      done = function(target)
        hint_handle = nil
        mode.cursor.line = mode.top + target.line
        mode.cursor.col = target.col
        if state == mode then
          settle(neotmux.pane.rect(mode.pane) or rect)
        end
      end,
      cancel = function()
        hint_handle = nil
      end,
    })
  end,
  ["cancel"] = function()
    leave()
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
  ["w"] = "word-forward",
  ["b"] = "word-back",
  ["e"] = "word-end",
  ["0"] = "line-start",
  ["^"] = "line-first-word",
  ["$"] = "line-end",
  ["C-u"] = "halfpage-up",
  ["C-d"] = "halfpage-down",
  ["g"] = "history-top",
  ["G"] = "history-bottom",
  ["v"] = "begin-selection",
  ["y"] = "copy-selection",
  ["f"] = "hint-jump",
  ["Esc"] = "cancel",
  ["q"] = "cancel",
}

-- The mode opens where the pane already is: the view is the visible screen and the cursor is
-- where the pane's own cursor is, which is where whatever you were typing is.
function M.enter()
  local pane = neotmux.pane.current()
  if pane == nil then
    return
  end
  -- A hint session over this pane ends here, before the snapshot is taken: the mode is about
  -- to take both the pane and the keyboard, and labels left drawn over a pane it has claimed
  -- could be neither read for what they are nor typed at. Ending it first also hands back
  -- whatever freeze it was holding, so the freeze this mode takes below is its own.
  hints.cancel()

  local buffer = snapshot.take(pane)
  if buffer == nil then
    return
  end
  if mode_handle then
    mode_handle:lower()
  end
  -- Held still for the life of the mode. The buffer above is the text the cursor is dragged
  -- over; this is the same instant's cells, and the two have to be the same instant or the
  -- selection is over one thing and the screen shows another. `freeze` answers `true` for any
  -- pane that exists rather than for one it changed, so whether this mode is the one holding
  -- the pane still is a question only `frozen` can answer, and only before the freeze.
  local froze = not neotmux.pane.frozen(pane) and neotmux.pane.freeze(pane) or false
  local at = neotmux.pane.cursor(pane) or { x = 0, y = 0 }
  state = {
    pane = pane,
    froze = froze,
    buffer = buffer,
    top = buffer.history,
    cursor = { line = buffer.history + at.y, col = at.x },
    anchor = nil,
  }
  mode_handle = modes.raise("copy")
end

neotmux.on_key(function(key)
  if state == nil then
    return false
  end
  if modes.current() ~= "copy" then
    -- Something is raised over this mode — a pick, at the time of writing — and every key
    -- belongs to it until it is done, not to this mode's own bindings, which would otherwise
    -- swallow the label being typed before `neotmux.hints`'s own `on_key`, registered after
    -- this one, ever saw it. Asked of the mode stack rather than of `hint_handle`, because
    -- what is up need not be the session this mode opened: `C-b f` opens one over the top of
    -- ours, and a test for our own handle would go back to swallowing keys the moment it did.
    return false
  end
  local rect = neotmux.pane.rect(state.pane)
  if rect == nil then
    -- The pane went out from under the mode. Leave, and hand the key on rather than eating it.
    leave()
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
    leave()
  end
end)

neotmux.on_event("focus_changed", function(event)
  if state and state.pane ~= event.pane then
    leave()
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

  -- The view is the pane's own cells, wound back into its own history: `ctx:view` says which
  -- rows this frame draws and the host snapshots them, so every colour and attribute the child
  -- wrote is still there.
  --
  -- It used to redraw the rows itself, one `ctx:rect` and one `ctx:text` each, out of the
  -- lines it had taken — and a line out of the ABI is text, so the view was monochrome. That
  -- was the cost of the host never scrolling a pane; scrolling one for the length of a frame
  -- is cheaper than redrawing it and is the only way to keep what it looked like.
  --
  -- The pane is frozen for the life of the mode, so what is drawn cannot move under the
  -- selection: the buffer the cursor is dragged over and the cells on the screen are the same
  -- instant.
  ctx:view(state.pane, math.max(0, state.buffer.history - state.top))

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

-- A sidebar down the left: every session, every window of the one you are in, every window's
-- panes, and what each pane is running. `C-b b` folds it to the markers column and back, and
-- `C-b s` moves to the next session, when the prefix is loaded.
--
-- Off by default, unlike the status bar: a status line spends a row nobody was using for
-- anything else, while a sidebar is an opinion about screen real estate — twenty-four columns
-- is a third of an eighty-column terminal. Turn it on from your own `init.lua`:
--
--   require('neotmux.defaults').sidebar = true
--
-- Nothing here is a host feature. `neotmux.reserve('left', n)` takes the band,
-- `handle:resize_to(n)` folds it, `ctx:reserved()` answers where it landed, and the rest is
-- `window.list`, `window.panes`, `pane.info` and `pane.current` read from a tick.
local segments = require("status.segments")

local M = {}

-- The two states the band is ever in, both read when it is swapped, so assigning to either
-- from your own `init.lua` takes effect on the next fold.
--
-- Closed is a rail rather than nothing: one column is the marker column, so folded you still
-- see how many windows and panes there are and which of them you are in — you just cannot
-- read their names. A band of nothing would be a band you have to remember is there.
M.widths = {
  open = 24,
  closed = 1,
}

-- Catppuccin Mocha, the palette the status bar was drawn from. Every field is read on the
-- tick, so assigning to one from your own init.lua takes effect.
M.colors = {
  bar = "#181825", -- mantle: the band everything else sits on
  session = "#9399b2", -- overlay2: a session you are not in
  current_session = "#f9e2af", -- yellow, so a session reads as a different kind of row
  window = "#cdd6f4",
  current_window = "#cba6f7", -- mauve, the status bar's accent for the window being shown
  pane = "#6c7086", -- overlay0: a pane in a window you are not looking at
  focused_pane = "#a6e3a1", -- green
  -- The right-hand column, dimmer than every label it sits beside: what a thing is running is
  -- what you scan for, not what you read first.
  command = "#585b70", -- surface2
  focused_command = "#6c7086", -- overlay0, a step up for the row you are on
}

-- The column of markers down the left, so a row says what it is before it says which one.
-- One cell each, and never a combining or double-width glyph: a marker that measures two
-- columns would push every name on that row out by one.
M.markers = {
  current_session = "\u{25cf}", -- ●
  session = "\u{25cb}", -- ○
  current_window = "\u{25b8}", -- ▸
  window = " ",
  focused_pane = "\u{2022}", -- •
  pane = " ",
}

M.band = neotmux.reserve("left", M.widths.open)

-- Which of the two the band is in. The rows are the same either way: `segments.draw` cuts a
-- run that does not fit, so the rail is the open sidebar with everything but the markers
-- clipped off, and there is no second way of drawing this to keep in step with the first.
M.open = true

-- Swap the two. A resize keeps the band's place on the left edge, which a remove and a fresh
-- reservation would not: folding would walk the sidebar inside anybody who reserved after it.
function M.toggle()
  -- Nil if the left edge was already spoken for when this file loaded.
  if not M.band then
    return
  end
  local want = M.open and M.widths.closed or M.widths.open
  if M.band:resize_to(want) then
    M.open = not M.open
  end
end

-- The prefix is the standard plugin and this one is optional, so this is the direction the
-- knowledge goes in: bound only if the prefix is already loaded, which it is by the time the
-- sidebar loads unless it was turned off.
if package.loaded["plugin.prefix"] then
  require("plugin.prefix").commands["b"] = M.toggle
end

-- The next session round, in the order `session.list` gives, which is name order. Not bound to
-- anything: `plugin.sessions` has `C-b s` and shows you what you are choosing between. This is
-- here for an `init.lua` that would rather step than pick.
function M.next_session()
  local names = neotmux.session.list()
  if #names < 2 then
    return
  end
  local current = neotmux.session.current()
  for at, name in ipairs(names) do
    if name == current then
      neotmux.session.switch(names[at % #names + 1])
      return
    end
  end
end

-- What a session is doing: how many panes it has, and what the one somebody is looking at is
-- running. Read through `session.with`, because a session that is not the one this tick is for
-- is still a session this plugin can see — which is the whole reason the sidebar can say this
-- at all.
local function doing(name)
  local panes, command = 0, nil
  neotmux.session.with(name, function()
    panes = #neotmux.pane.list()
    local focus = neotmux.pane.current()
    local info = focus and neotmux.pane.info(focus)
    command = info and info.command
  end)
  return panes, command
end

-- What a pane is running, or nil where the host has not got it. Every pane the session
-- holds, hidden windows included: `pane.info` answers for all of them.
local function command(pane)
  local info = neotmux.pane.info(pane)
  return info and info.command
end

-- What a pane row says when the host could not read the pane's command — a child that exited
-- between the sample and the tick, or a pane that closed while the rows were being built.
local UNKNOWN = "\u{2014}" -- em dash

-- One row: a marker in the first column, a label after it, and — where there is room — what
-- the thing is running, against the right edge.
--
-- The marker is alone in column zero on purpose. That column is the whole of the sidebar when
-- it is folded, so everything else starts at column two and the fold keeps meaning something.
-- The right column is what makes the rows read as a table rather than as a paragraph, and it
-- is the part that is dropped when a label is long: a name you cannot read is worse than a
-- command you have to widen the band for.
local function row(marker, indent, label, right, colour, right_colour)
  return function(width)
    local left = marker .. " " .. string.rep(" ", indent) .. label
    local runs = { { text = left, fg = colour, bg = M.colors.bar } }
    if right and right ~= "" then
      local room = width - segments.columns(left) - segments.columns(right) - 1
      if room >= 1 then
        runs[#runs + 1] = { text = string.rep(" ", room), bg = M.colors.bar }
        runs[#runs + 1] = {
          text = right,
          fg = right_colour or M.colors.command,
          bg = M.colors.bar,
        }
      end
    end
    return runs
  end
end

-- A row with nothing on it, which is how the two groups are told apart. Cheaper to read than a
-- rule, and it folds to nothing rather than to a stray glyph in the marker column.
local function gap()
  return function()
    return { { text = "", bg = M.colors.bar } }
  end
end

-- The whole sidebar as a list of rows, top to bottom. Built before anything is drawn so the
-- band can simply be filled until it runs out of rows.
-- Which row is worth keeping on screen when there are more rows than band: the pane you are
-- in, or failing that the window, or failing that the session. A sidebar that has scrolled the
-- thing you are looking at off the bottom is a sidebar about somebody else's session.
local anchor = 1

local function rows()
  local current_window = neotmux.window.current()
  local focused = neotmux.pane.current()
  local out = {}
  anchor = 1
  -- The sessions first, and every one of them, including the ones nobody is looking at: what
  -- each is running is the thing you came to the sidebar to find out.
  local here = neotmux.session.current()
  for _, name in ipairs(neotmux.session.list()) do
    local panes, running = doing(name)
    local is_here = name == here
    if is_here then
      anchor = #out + 1
    end
    out[#out + 1] = row(
      is_here and M.markers.current_session or M.markers.session,
      0,
      name,
      (running or UNKNOWN) .. "  " .. panes .. "p",
      is_here and M.colors.current_session or M.colors.session
    )
  end
  out[#out + 1] = gap()
  for window_at, window in ipairs(neotmux.window.list()) do
    local is_current = window == current_window
    if is_current then
      anchor = #out + 1
    end
    out[#out + 1] = row(
      is_current and M.markers.current_window or M.markers.window,
      0,
      tostring(window_at),
      nil,
      is_current and M.colors.current_window or M.colors.window
    )
    -- Nil for a window that closed between the list and now, which is not an error worth a
    -- sidebar with a hole in it.
    for pane_at, pane in ipairs(neotmux.window.panes(window) or {}) do
      local is_focused = pane == focused
      if is_focused then
        anchor = #out + 1
      end
      out[#out + 1] = row(
        is_focused and M.markers.focused_pane or M.markers.pane,
        1,
        window_at .. "." .. pane_at,
        command(pane) or UNKNOWN,
        is_focused and M.colors.focused_pane or M.colors.pane,
        is_focused and M.colors.focused_command or M.colors.command
      )
    end
  end
  return out
end

neotmux.on_tick(function(ctx)
  local band = ctx:reserved()
  -- Nil while the terminal is too narrow to spare the columns. The panes have them back, so
  -- there is nowhere to draw and nothing to draw over.
  if not band then
    return
  end
  ctx:rect(nil, band.x, band.y, band.w, band.h, { bg = M.colors.bar })

  -- More rows than band is a session, not a mistake. Which rows go is the question: the ones
  -- that do not fit are dropped from the top or the bottom so that `anchor` — the pane you are
  -- in — is always one of the ones that stayed.
  local all = rows()
  local first = 1
  if #all > band.h then
    first = math.min(math.max(anchor - math.floor(band.h / 2), 1), #all - band.h + 1)
  end
  for at = first, math.min(first + band.h - 1, #all) do
    segments.draw(ctx, band.x, band.y + at - first, band.w, all[at](band.w))
  end
end)

return M

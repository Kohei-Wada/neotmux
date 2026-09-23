-- A sidebar down the left: every session with what it is doing, then the windows and panes of
-- the one you are in. `C-b b` folds it to the markers column and back, when the prefix is
-- loaded.
--
-- Off by default, unlike the status bar: a status line spends a row nobody was using for
-- anything else, while a sidebar is an opinion about screen real estate. Turn it on from your
-- own `init.lua`:
--
--   require('neotmux.defaults').sidebar = true
--
-- Nothing here is a host feature. `neotmux.reserve('left', n)` takes the band,
-- `handle:resize_to(n)` widens and folds it, `ctx:reserved()` answers where it landed, and the
-- rest is `session.list`, `session.with`, `window.list`, `window.panes`, `pane.info` and
-- `neotmux.state` read from a tick.
--
-- The layout is a table, not a shape baked into this file. Each level draws one entry as a list
-- of rows, and each row is a list of tokens:
--
--   local sidebar = require('plugin.sidebar')
--   sidebar.sessions.rows = { { 'state_icon', 'name' }, { 'command', 'panes' } }
--   sidebar.panes.rows = { { 'state_icon', 'number', 'command' } }
--
-- so moving what is on which line, or dropping a token you do not care about, is an assignment
-- rather than a fork of this file.
local segments = require("status.segments")
local state = require("neotmux.state")

local M = {}

-- What the band is worth, in columns. The width is taken from what there is to show, between
-- these two, so a session called `feature/long-branch-name` is not cut to nothing — and `min`
-- is what it settles at when there is nothing long to show, which is most of the time. Wide
-- enough that a name and what it is running are both readable without the eye working for it.
M.widths = {
  min = 27,
  max = 36,
  -- What is left when it is folded: one column, so the markers still say how many sessions and
  -- panes there are and which of them you are in. A band of nothing would be a band you have to
  -- remember is there.
  closed = 1,
}

-- Catppuccin Mocha, the palette the status bar was drawn from. Every field is read on the tick,
-- so assigning to one from your own `init.lua` takes effect.
M.colors = {
  bar = "#181825", -- mantle: the band everything else sits on
  session = "#9399b2", -- overlay2: a session you are not in
  current_session = "#f9e2af", -- yellow, so a session reads as a different kind of row
  window = "#cdd6f4",
  current_window = "#cba6f7", -- mauve, the status bar's accent for the window being shown
  pane = "#6c7086", -- overlay0: a pane in a window you are not looking at
  focused_pane = "#a6e3a1", -- green
  -- The second row of an entry, and anything else that is detail rather than name.
  detail = "#585b70", -- surface2
  -- One per state, so the icon carries its meaning in the colour as well as the shape.
  working = "#a6e3a1", -- green
  blocked = "#f9e2af", -- yellow: the one that wants you
  idle = "#585b70", -- surface2
}

M.markers = {
  current_window = "\u{25b8}", -- ▸
  window = " ",
  focused_pane = "\u{2022}", -- •
  pane = " ",
}

-- A session holds windows and a window holds panes, so the sidebar draws what it is: a tree.
-- The guides go after the first column and never in it — that column is the whole of the band
-- when it is folded, and a fold showing a run of `│` says nothing about where you are.
--
--   require('plugin.sidebar').tree.on = false   -- indent without the lines
M.tree = {
  on = true,
  -- Two columns each, so a level always costs the same and the labels line up whichever
  -- branch they are on.
  glyphs = {
    branch = "\u{251c}\u{2500}", -- ├─
    last = "\u{2514}\u{2500}", -- └─
    through = "\u{2502} ", -- │
    blank = "  ",
  },
}

-- The guide in front of a row, given where it sits: one segment per level above it saying
-- whether that level had anything after it, and one for the row itself.
local function guide(levels)
  if not M.tree.on then
    return string.rep("  ", #levels)
  end
  local out = {}
  for at, last in ipairs(levels) do
    if at == #levels then
      out[#out + 1] = last and M.tree.glyphs.last or M.tree.glyphs.branch
    else
      out[#out + 1] = last and M.tree.glyphs.blank or M.tree.glyphs.through
    end
  end
  return table.concat(out)
end

-- Each level's layout: the rows of one entry, and the blank rows between entries. A row that
-- comes out empty is dropped rather than drawn, so a second row of detail costs nothing on a
-- session with no detail to give.
M.sessions = {
  rows = { { "state_icon", "name" }, { "command", "panes" } },
  row_gap = 0,
}
M.windows = {
  rows = { { "marker", "tree", "number", "name" } },
  row_gap = 0,
}
M.panes = {
  rows = { { "marker", "tree", "number", "command" } },
  row_gap = 0,
}

-- What a token is worth, given what the row is about. Add one and it is available to every
-- layout at that level:
--
--   require('plugin.sidebar').tokens.sessions.age = function(it) ... end
M.tokens = {
  sessions = {
    state_icon = function(it)
      return state.icon(it.state), M.colors[it.state]
    end,
    name = function(it)
      return it.name, it.here and M.colors.current_session or M.colors.session
    end,
    command = function(it)
      return it.command, M.colors.detail
    end,
    panes = function(it)
      return it.panes .. "p", M.colors.detail
    end,
  },
  windows = {
    tree = function(it)
      return it.guide, M.colors.detail
    end,
    marker = function(it)
      return it.current and M.markers.current_window or M.markers.window,
        it.current and M.colors.current_window or M.colors.window
    end,
    number = function(it)
      return tostring(it.number), it.current and M.colors.current_window or M.colors.window
    end,
    -- Nothing for a window nobody has named: its number is what it is called until then.
    name = function(it)
      return it.name, it.current and M.colors.current_window or M.colors.window
    end,
  },
  panes = {
    tree = function(it)
      return it.guide, M.colors.detail
    end,
    marker = function(it)
      return it.focused and M.markers.focused_pane or M.markers.pane,
        it.focused and M.colors.focused_pane or M.colors.pane
    end,
    state_icon = function(it)
      return state.icon(it.state), M.colors[it.state]
    end,
    number = function(it)
      return it.number, it.focused and M.colors.focused_pane or M.colors.pane
    end,
    command = function(it)
      return it.command, M.colors.detail
    end,
  },
}

M.band = neotmux.reserve("left", M.widths.min)

-- Which of the two the band is in. The rows are the same either way: `segments.draw` cuts a run
-- that does not fit, so the rail is the open sidebar with everything but the first column
-- clipped off, and there is no second way of drawing this to keep in step with the first.
M.open = true

-- What a pane is running, or nil where the host has not got it — a child that exited between
-- the sample and the tick, or a pane that closed while the rows were being built.
local UNKNOWN = "\u{2014}" -- em dash

local function command_of(pane)
  local info = neotmux.pane.info(pane)
  return info and info.command
end

-- What a session is doing, read through `session.with`: how many panes it has, what the one
-- somebody is looking at is running, and whether any of them is waiting for you. A session that
-- is not the one this tick is for is still a session this plugin can see, which is the whole
-- reason the sidebar can say any of this.
local function about(name)
  local panes, command, worst = 0, nil, "idle"
  neotmux.session.with(name, function()
    local list = neotmux.pane.list()
    panes = #list
    local focus = neotmux.pane.current()
    command = focus and command_of(focus)
    worst = state.worst(list)
  end)
  return { panes = panes, command = command or UNKNOWN, state = worst }
end

-- One row: every token that has something to say, in order, separated by a space. A token that
-- answers nil or an empty string is left out, so a row whose tokens are all empty is no row.
local function build(level, it)
  local runs = {}
  for _, name in ipairs(level.rows_in_hand) do
    local token = M.tokens[level.kind][name]
    if token then
      local text, colour = token(it)
      if text and text ~= "" then
        if #runs > 0 then
          runs[#runs + 1] = { text = " ", bg = M.colors.bar }
        end
        runs[#runs + 1] = { text = text, fg = colour or M.colors.detail, bg = M.colors.bar }
      end
    end
  end
  return runs
end

-- Every row of one entry, indented past the first column for all but the first: the first
-- column is the whole of the sidebar when it is folded, so what is in it has to be the marker
-- and the rest has to start after it.
local function entry(level, kind, it, out)
  for at, tokens in ipairs(level.rows) do
    level.rows_in_hand = tokens
    level.kind = kind
    local runs = build(level, it)
    if #runs > 0 then
      if at > 1 then
        table.insert(runs, 1, { text = "  ", bg = M.colors.bar })
      end
      out[#out + 1] = runs
    end
  end
  for _ = 1, level.row_gap or 0 do
    out[#out + 1] = { { text = "", bg = M.colors.bar } }
  end
end

-- Which row is worth keeping on screen when there are more rows than band: the pane you are in,
-- or failing that the window, or failing that the session. A sidebar that has scrolled the
-- thing you are looking at off the bottom is a sidebar about somebody else's session.
local anchor = 1

-- The windows and panes of the session that is open, hung under it: a window belongs to a
-- session and a pane to a window, so the list of them is drawn where that says they are.
local function subtree(out)
  local current_window = neotmux.window.current()
  local focused = neotmux.pane.current()
  local windows = neotmux.window.list()
  for window_at, window in ipairs(windows) do
    local current = window == current_window
    local last_window = window_at == #windows
    if current then
      anchor = #out + 1
    end
    entry(M.windows, "windows", {
      number = window_at,
      name = neotmux.window.name(window),
      current = current,
      guide = guide({ last_window }),
    }, out)
    -- Nil for a window that closed between the list and now, which is not an error worth a
    -- sidebar with a hole in it.
    local panes = neotmux.window.panes(window) or {}
    for pane_at, pane in ipairs(panes) do
      local is_focused = pane == focused
      if is_focused then
        anchor = #out + 1
      end
      entry(M.panes, "panes", {
        number = window_at .. "." .. pane_at,
        focused = is_focused,
        command = command_of(pane) or UNKNOWN,
        state = state.of(pane) or "idle",
        guide = guide({ last_window, pane_at == #panes }),
      }, out)
    end
  end
end

local function rows()
  local out = {}
  anchor = 1
  local here = neotmux.session.current()
  for _, name in ipairs(neotmux.session.list()) do
    local it = about(name)
    it.name = name
    it.here = name == here
    if it.here then
      anchor = #out + 1
    end
    entry(M.sessions, "sessions", it, out)
    -- Under the one you are in, because that is where its windows are. The others are named
    -- and no more: what is inside a session you are not in is what the picker is for.
    if it.here then
      subtree(out)
    end
  end
  return out
end

-- How wide the band wants to be: the widest row there is, between the two bounds. Taken from
-- what is on it rather than fixed, so a sidebar of short names is a narrow sidebar.
local function width_for(all)
  local widest = 0
  for _, runs in ipairs(all) do
    widest = math.max(widest, segments.width(runs))
  end
  return math.max(M.widths.min, math.min(M.widths.max, widest + 1))
end

-- Swap the two. A resize keeps the band's place on the left edge, which a remove and a fresh
-- reservation would not: folding would walk the sidebar inside anybody who reserved after it.
function M.toggle()
  -- Nil if the left edge was already spoken for when this file loaded.
  if not M.band then
    return
  end
  M.open = not M.open
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

-- The prefix is the standard plugin and this one is optional, so this is the direction the
-- knowledge goes in: bound only if the prefix is already loaded, which it is by the time the
-- sidebar loads unless it was turned off.
if package.loaded["plugin.prefix"] then
  require("plugin.prefix").commands["b"] = M.toggle
end

neotmux.on_tick(function(ctx)
  local band = ctx:reserved()
  -- Nil while the terminal is too narrow to spare the columns. The panes have them back, so
  -- there is nowhere to draw and nothing to draw over.
  if not band then
    return
  end
  local all = rows()

  -- The width the content wants, asked for once it is known. `resize_to` lands on the next
  -- frame, which is why this is a comparison and not a call every tick.
  if M.band then
    local want = M.open and width_for(all) or M.widths.closed
    if want ~= band.w then
      M.band:resize_to(want)
    end
  end

  ctx:rect(nil, band.x, band.y, band.w, band.h, { bg = M.colors.bar })

  -- More rows than band is the ordinary case, and which rows go is the question: the ones that
  -- do not fit are dropped from the top or the bottom so that `anchor` — the pane you are in —
  -- is always one of the ones that stayed.
  local first = 1
  if #all > band.h then
    first = math.min(math.max(anchor - math.floor(band.h / 2), 1), #all - band.h + 1)
  end
  for at = first, math.min(first + band.h - 1, #all) do
    segments.draw(ctx, band.x, band.y + at - first, band.w, all[at])
  end
end)

return M

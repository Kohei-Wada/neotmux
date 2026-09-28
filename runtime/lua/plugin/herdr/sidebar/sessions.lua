-- The session tree, the first section of `plugin.herdr.sidebar`: every session with what it is
-- doing, then the windows and panes of the one you are in.
--
-- The layout is a table, not a shape baked into this file. Each level draws one entry as a list
-- of rows, and each row is a list of tokens:
--
--   local sessions = require('plugin.herdr.sidebar.sessions')
--   sessions.sessions.rows = { { 'state_icon', 'name' }, { 'command', 'panes' } }
--   sessions.panes.rows = { { 'state_icon', 'number', 'command' } }
--
-- so moving what is on which line, or dropping a token you do not care about, is an assignment
-- rather than a fork of this file. The colours are the sidebar's: `style.get('sidebar')`.
local state = require("neotmux.state")
local style = require("neotmux.style")

local M = {}

-- The sidebar's colours, set by `build` for the tokens, which are called with nothing but the
-- row they are about.
local colors

M.markers = {
  current_window = "\u{25b8}", -- ▸
  window = " ",
  focused_pane = "\u{2022}", -- •
  pane = " ",
}

-- A session holds windows and a window holds panes, so the tree draws what it is. The guides go
-- after the first column and never in it — that column is the whole of the band when it is
-- folded, and a fold showing a run of `│` says nothing about where you are.
--
--   require('plugin.herdr.sidebar.sessions').tree.on = false   -- indent without the lines
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
--   require('plugin.herdr.sidebar.sessions').tokens.sessions.age = function(it) ... end
M.tokens = {
  sessions = {
    state_icon = function(it)
      return state.icon(it.state), colors[it.state]
    end,
    name = function(it)
      return it.name, it.here and colors.current_session or colors.session
    end,
    command = function(it)
      return it.command, colors.detail
    end,
    panes = function(it)
      return it.panes .. "p", colors.detail
    end,
  },
  windows = {
    tree = function(it)
      return it.guide, colors.detail
    end,
    marker = function(it)
      return it.current and M.markers.current_window or M.markers.window,
        it.current and colors.current_window or colors.window
    end,
    number = function(it)
      return tostring(it.number), it.current and colors.current_window or colors.window
    end,
    -- Nothing for a window nobody has named: its number is what it is called until then.
    name = function(it)
      return it.name, it.current and colors.current_window or colors.window
    end,
  },
  panes = {
    tree = function(it)
      return it.guide, colors.detail
    end,
    marker = function(it)
      return it.focused and M.markers.focused_pane or M.markers.pane,
        it.focused and colors.focused_pane or colors.pane
    end,
    state_icon = function(it)
      return state.icon(it.state), colors[it.state]
    end,
    number = function(it)
      return it.number, it.focused and colors.focused_pane or colors.pane
    end,
    command = function(it)
      return it.command, colors.detail
    end,
  },
}

-- What a pane is running, or nil where the host has not got it — a child that exited between
-- the sample and the tick, or a pane that closed while the rows were being built.
local UNKNOWN = "\u{2014}" -- em dash

local function command_of(pane)
  local info = neotmux.pane.info(pane)
  return info and info.command
end

-- What a session is doing, read through `session.with`: how many panes it has, what the one
-- somebody is looking at is running, and whether any of them is waiting for you.
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

-- Every row of one entry, indented past the first column for all but the first: the first
-- column is the whole of the sidebar when it is folded, so what is in it has to be the marker
-- and the rest has to start after it.
local function entry(sidebar, level, kind, it, out)
  for at, names in ipairs(level.rows) do
    local runs = sidebar.row(M.tokens[kind], names, it)
    if #runs > 0 then
      if at > 1 then
        table.insert(runs, 1, { text = "  ", bg = colors.bar })
      end
      out[#out + 1] = runs
    end
  end
  for _ = 1, level.row_gap or 0 do
    out[#out + 1] = { { text = "", bg = colors.bar } }
  end
end

-- The windows and panes of the session that is open, into `out`. Answers the row worth keeping
-- on screen: the pane you are in, or failing that the window, or nil when there is neither.
-- `plugin.herdr.sidebar.spaces` hangs the same rows under the space you are in, laid out by its own
-- `levels`: `{ windows = ..., panes = ... }`, shaped as `M.windows` and `M.panes` are.
function M.subtree(sidebar, out, levels)
  colors = style.get("sidebar")
  levels = levels or M
  local anchor
  local current_window = neotmux.window.current()
  local focused = neotmux.pane.current()
  local windows = neotmux.window.list()
  for window_at, window in ipairs(windows) do
    local current = window == current_window
    local last_window = window_at == #windows
    if current then
      anchor = #out + 1
    end
    entry(sidebar, levels.windows, "windows", {
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
      entry(sidebar, levels.panes, "panes", {
        number = window_at .. "." .. pane_at,
        focused = is_focused,
        command = command_of(pane) or UNKNOWN,
        state = state.of(pane) or "idle",
        guide = guide({ last_window, pane_at == #panes }),
      }, out)
    end
  end
  return anchor
end

-- Beside the rows, what a fit needs to cut them sensibly: each session's own rows and which one
-- is here, and the subtree hung under it with the row inside it worth keeping.
function M.build(sidebar)
  colors = style.get("sidebar")
  local rows, entries, anchor = {}, {}, 1
  local tree = { rows = {} }
  local here = neotmux.session.current()
  for _, name in ipairs(neotmux.session.list()) do
    local it = about(name)
    it.name = name
    it.here = name == here
    local own = {}
    entry(sidebar, M.sessions, "sessions", it, own)
    entries[#entries + 1] = { rows = own, here = it.here }
    if it.here then
      anchor = #rows + 1
    end
    for _, runs in ipairs(own) do
      rows[#rows + 1] = runs
    end
    -- Under the one you are in, because that is where its windows are. The others are named
    -- and no more: what is inside a session you are not in is what the picker is for.
    if it.here then
      tree.anchor = M.subtree(sidebar, tree.rows)
      if tree.anchor then
        anchor = #rows + tree.anchor
      end
      for _, runs in ipairs(tree.rows) do
        rows[#rows + 1] = runs
      end
    end
  end
  return { rows = rows, anchor = anchor, entries = entries, tree = tree }
end

-- What stands in for the subtree rows cut off either side of the pane you are in. Indented
-- like a tree row, so the first column stays the markers'.
local function more(n)
  return { { text = "  \u{2026} " .. n .. " more", fg = colors.detail, bg = colors.bar } }
end

-- Sessions before panes. Every row if they fit; else every session's rows with the subtree of
-- the one you are in cut to what is left around the focused pane; else each session at its first
-- row and no subtree; else those first rows scrolled around the one you are in. Where a session
-- is, is the thing the sidebar is for, and the panes of the one you are in are on your screen.
function M.fit(model, height, sidebar)
  colors = style.get("sidebar")
  if #model.rows <= height then
    return model.rows
  end
  local heads = 0
  for _, it in ipairs(model.entries) do
    heads = heads + #it.rows
  end
  -- A row that is only there to keep the entries even goes before any of the tree does.
  if #model.tree.rows > 0 and heads >= height then
    local entries = {}
    heads = 0
    for _, it in ipairs(model.entries) do
      local rows = {}
      for _, runs in ipairs(it.rows) do
        if not runs.blank then
          rows[#rows + 1] = runs
        end
      end
      entries[#entries + 1] = { rows = rows, here = it.here }
      heads = heads + #rows
    end
    model = { rows = model.rows, entries = entries, tree = model.tree }
  end
  if heads <= height then
    local cut = sidebar.scroll(model.tree, height - heads, { above = more, below = more })
    local out = {}
    for _, it in ipairs(model.entries) do
      for _, runs in ipairs(it.rows) do
        out[#out + 1] = runs
      end
      if it.here then
        for _, runs in ipairs(cut) do
          out[#out + 1] = runs
        end
      end
    end
    return out
  end
  local firsts, here = {}, 1
  for _, it in ipairs(model.entries) do
    if #it.rows > 0 then
      firsts[#firsts + 1] = it.rows[1]
      if it.here then
        here = #firsts
      end
    end
  end
  return sidebar.scroll({ rows = firsts, anchor = here }, height)
end

return M

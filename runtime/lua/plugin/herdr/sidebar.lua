-- A sidebar down the left: a stack of sections, each a table saying what rows it has and how to
-- cut them to a height. The shipped ones are herdr's: every session, `plugin.herdr.sidebar.spaces`,
-- and under it every agent in every session, `plugin.herdr.sidebar.agents`.
-- `C-b b` folds the band to its first column and back, when the prefix is loaded.
--
-- Off by default, unlike the status bar: a status line spends a row nobody was using for
-- anything else, while a sidebar is an opinion about screen real estate. Turn it on from your
-- own `init.lua`:
--
--   require('neotmux.defaults').sidebar = true
--
-- What is in it is `sections`, top to bottom, a list you assign to like any other field here:
--
--   table.insert(require('plugin.herdr.sidebar').sections, {
--     title = 'mine',   -- a header above it; nil for none
--     share = 0.3,      -- the most of the band it takes when there is not room for everybody
--     always = true,    -- headed even with nothing to show
--     build = function(sidebar) return { rows = { runs, ... }, anchor = 1 } end,
--     fit = function(model, height, sidebar) return { runs, ... } end,   -- optional
--     rail = function(sidebar) return { runs, ... } end,   -- optional: what it is folded
--     patch = function(sidebar, model, change) ... end,   -- optional: see `patch` below
--   })
--
-- A section with no `patch` has the whole band laid out again whenever any pane's state moves.
--
-- A row is a list of runs, `{ text = ..., hl = 'sidebar:branch', bold = ... }`, each coloured by
-- the highlight group it names, and a row with a `fill` has the band up to its rule painted
-- that group's colour. `build` is handed this module
-- rather than requiring it, because the shipped sections are required from here while this file
-- is still loading. Without a `fit`, a section is cut by `sidebar.scroll`.
--
-- Nothing here is a host feature. `neotmux.reserve('left', n)` takes the band,
-- `handle:resize_to(n)` widens and folds it, and `ctx:reserved()` answers where it landed. The
-- rows are written into a buffer of each session's that the band shows in that session
-- (`neotmux.buf`), rewritten only when something they show changed: see `dirty` below.
local segments = require("status.segments")
local style = require("neotmux.style")
local seen_state = require("neotmux.seen")
local g = neotmux.group("plugin.herdr.sidebar")

local M = {}

-- What the band is worth, in columns. The width is taken from what there is to show, between
-- these two, so a session called `feature/long-branch-name` is not cut to nothing — and `min`
-- is what it settles at when there is nothing long to show, which is most of the time.
M.widths = {
  min = 26,
  max = 26,
  -- herdr's rail: a number and a state a row, which is what is left of a session or an agent
  -- when there is no room for its name.
  closed = 4,
}

-- Catppuccin Mocha, the palette the status bar was drawn from. `neotmux.style` publishes each
-- part as the highlight group `sidebar:<part>` the rows name, so restyling it with
-- `neotmux.style.set('sidebar', ...)` or a theme change recolours the band with nothing rewritten.
-- `bar` has no default: the terminal's own background, which is herdr's.
style.default("sidebar", {
  text = "text",
  subtext = "subtext0", -- a name you are not at
  overlay = "overlay0", -- headers, and what is detail rather than name
  branch = "mauve", -- the branch of the session you are in
  -- The rows you are at: the session you are in and the agent you are focused on.
  here = "base",
  header = "overlay0",
  session = "overlay2", -- a session you are not in
  current_session = "yellow", -- so a session reads as a different kind of row
  window = "text",
  current_window = "mauve", -- the status bar's accent for the window being shown
  pane = "overlay0", -- a pane in a window you are not looking at
  focused_pane = "green",
  -- The second row of an entry, and anything else that is detail rather than name.
  detail = "surface2",
  rule = "base", -- the band's right edge and the line between sections
  -- One per state, so the icon carries its meaning in the colour as well as the shape.
  -- herdr's, so a herdr user reads them without learning them again.
  working = "yellow",
  blocked = "red", -- the one that wants you
  done = "teal", -- finished while you were not looking (`neotmux.seen`)
  idle = "green",
  unknown = "overlay0", -- a session with no agent in it, which has no state to say
  -- The navigate cursor (`C-b w`, `C-b a`).
  selection = "surface0",
})

-- One per state, herdr's `dots`: the colour says which, the shape only whether anything is
-- going on. herdr's `symbols` are one assignment away:
--
--   require('plugin.herdr.sidebar').icons =
--     { working = '◐', blocked = '×', done = '✓', idle = '○' }
M.icons = {
  working = "\u{25cf}", -- ●
  blocked = "\u{25cf}",
  done = "\u{25cf}",
  idle = "\u{25cb}", -- ○
  unknown = "\u{b7}", -- ·
}

function M.icon(state)
  return M.icons[state] or M.icons.unknown
end

-- The column down the band's right edge that says where the panes start.
M.rule = "\u{2502}" -- │

M.band = g.reserve("left", M.widths.min)

local ns = neotmux.buf.namespace("plugin.herdr.sidebar")

-- Which of the two the band is in. Folded, a section with a `rail` draws that, and one without is
-- its open rows cut to the rail's width.
M.open = true

-- Where the band is cut between two sections, as a part of its height; herdr cuts at half. Nil
-- shares it out by what each section has, which is also what three or more sections get.
M.split = 0.5

-- The bottom row's corner: what `C-b b` would do. Nil gives the row back to the sections.
M.folds = { open = "\u{ab}", closed = "\u{bb}" } -- «, »

-- One row: every token in `names` that has something to say, in order, separated by a space.
-- A token answers its text, a highlight group name, whether it is bold and whether it gives up
-- its end to an ellipsis when the row is too wide for the band. One that answers nil or an empty
-- string is left out, so a row whose tokens are all empty is no row.
function M.row(tokens, names, it)
  local runs = {}
  for _, name in ipairs(names) do
    local token = tokens[name]
    if token then
      local text, group, bold, shrink = token(it)
      if text and text ~= "" then
        if #runs > 0 then
          runs[#runs + 1] = { text = " " }
        end
        runs[#runs + 1] =
          { text = text, hl = group or "sidebar:detail", bold = bold, shrink = shrink }
      end
    end
  end
  return runs
end

-- What `scroll` puts where it cut rows: `n` counts the row the mark itself stands in, so the
-- number is how many rows are not on screen on that side.
local function arrow(glyph)
  return function(n)
    return { { text = glyph .. " " .. n, hl = "sidebar:detail" } }
  end
end

M.marks = { above = arrow("\u{2191}"), below = arrow("\u{2193}") }

-- The default fit: every row if they fit, and otherwise a window of them centred on `anchor`
-- and clamped to the ends, with the first and last rows of the window given over to saying how
-- much was cut on that side. A mark never stands in for the anchor, so the row that matters is
-- always one of the ones that stayed.
function M.scroll(model, height, marks)
  marks = marks or M.marks
  local rows = model.rows
  if height <= 0 then
    return {}
  end
  if #rows <= height then
    return rows
  end
  local anchor = model.anchor or 1
  local first = math.min(math.max(anchor - math.floor(height / 2), 1), #rows - height + 1)
  local last = first + height - 1
  local out = {}
  for at = first, last do
    out[#out + 1] = rows[at]
  end
  if first > 1 and first ~= anchor then
    out[1] = marks.above(first)
  end
  if last < #rows and last ~= anchor then
    out[#out] = marks.below(#rows - last + 1)
  end
  return out
end

-- `fit` is called with the sidebar as its third argument, which `scroll` would take for marks.
local function default_fit(model, height)
  return M.scroll(model, height)
end

-- One broken section must not take the sidebar down with it, and must say which one it was —
-- once, because it will raise again on every frame after this one.
local complained = setmetatable({}, { __mode = "k" })

local function guarded(section, at, what, fn, ...)
  local ok, result = pcall(fn, ...)
  if ok then
    return result
  end
  if not complained[section] then
    complained[section] = true
    local which = "plugin.herdr.sidebar: section '" .. tostring(section.title or at) .. "' " .. what
    neotmux.notify("error", which .. ": " .. tostring(result))
  end
  return nil
end

-- How many rows each live section gets. All of what it wants when everybody fits; otherwise a
-- capped section is held to its share first, the uncapped ones take what they want of the rest
-- in order, and whatever is still left goes back to the capped ones in order.
local function share_out(live, height)
  local wants, total = {}, 0
  for i, it in ipairs(live) do
    wants[i] = #it.model.rows + it.heading
    total = total + wants[i]
  end
  if total <= height then
    return wants
  end
  local got, left = {}, height
  for i, it in ipairs(live) do
    if it.section.share then
      got[i] = math.min(wants[i], math.floor(height * it.section.share), left)
      left = left - got[i]
    end
  end
  for i, it in ipairs(live) do
    if not it.section.share then
      got[i] = math.min(wants[i], left)
      left = left - got[i]
    end
  end
  for i, it in ipairs(live) do
    if it.section.share then
      local more = math.min(wants[i] - got[i], left)
      got[i] = got[i] + more
      left = left - more
    end
  end
  return got
end

-- ` agents          grouped`: the title, and on the right whatever the section's `aside` says —
-- a string, or a function asked each frame.
local function header(section, width)
  local aside = section.aside
  if type(aside) == "function" then
    aside = aside()
  end
  local left = " " .. section.title
  local right = aside or ""
  local gap = width - segments.columns(left) - segments.columns(right)
  if gap < 1 then
    right, gap = "", 0
  end
  return {
    { text = left, hl = "sidebar:header", bold = true },
    { text = string.rep(" ", gap) },
    { text = right, hl = "sidebar:header", bold = true },
  }
end

-- The rows a section's header takes: the line between it and the section above, then its title
-- and a blank under it. The first section has no line above it; a rail has no titles.
local function heading(section, first, rail)
  local rows = first and 0 or 1
  if section.title and not rail then
    rows = rows + 2
  end
  return rows
end

-- A row too wide for `width` ends its first `shrink` run in an ellipsis, so the runs after it
-- stay on screen. One that cannot give up enough is left to be cut at the band.
local function shrunk(runs, width)
  local over = segments.width(runs) - width
  if over <= 0 then
    return runs
  end
  for at, run in ipairs(runs) do
    if run.shrink then
      local keep = segments.columns(run.text) - over - 1
      if keep < 1 then
        return runs
      end
      local out = { fill = runs.fill }
      for k, other in ipairs(runs) do
        out[k] = other
      end
      out[at] = setmetatable(
        { text = segments.clip(run.text, keep) .. "\u{2026}" },
        { __index = run }
      )
      return out
    end
  end
  return runs
end

-- One row of the grid: its runs cut to `width` as `shrunk` cuts them, from column `x`, as a line
-- of text and the extmarks that colour it.
local function put(grid, y, x, width, runs)
  runs = shrunk(runs, width)
  local row = grid[y]
  for _, run in ipairs(runs) do
    local room = width - (row.cols - x)
    if room <= 0 then
      break
    end
    local text = run.text or ""
    if segments.columns(text) > room then
      text = segments.clip(text, room)
    end
    if text ~= "" then
      local at = row.cols
      row.text = row.text .. text
      row.cols = row.cols + segments.columns(text)
      if run.hl then
        row.marks[#row.marks + 1] = { col = at, end_col = row.cols, hl = run.hl, bold = run.bold }
      end
    end
  end
  if runs.fill then
    row.fill = runs.fill
  end
end

local function pad_to(row, col)
  if row.cols < col then
    row.text = row.text .. string.rep(" ", col - row.cols)
    row.cols = col
  end
end

-- How many rows each live section gets out of `height`: cut at `M.split` when there are two,
-- shared out by what they have otherwise.
local function heights_of(live, height)
  if M.split and #live == 2 then
    local top = math.min(height, math.floor((height + 1) * M.split))
    return { top, height - top }
  end
  return share_out(live, height)
end

-- How wide the band wants to be for a widest row of `widest` columns, between the two bounds.
local function width_at(widest)
  -- One for a column of air, one for the rule.
  return math.max(M.widths.min, math.min(M.widths.max, widest + 2))
end

-- How wide the band wants to be: the widest row any section has, and that widest.
local function width_for(live)
  local widest = 0
  for _, it in ipairs(live) do
    for _, runs in ipairs(it.model.rows) do
      widest = math.max(widest, segments.width(runs))
    end
  end
  return width_at(widest), widest
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
-- anything: `plugin.tmux.sessions` has `C-b s` and shows you what you are choosing between. This is
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
-- knowledge goes in.
require("neotmux.bind").prefix("b", M.toggle, "fold the sidebar to its rail, or open it")

-- A row once everything is in it: the band's own colour where a fill of its own did not paint,
-- named even while unset so a `bar` set later paints at once; then the rule down the right edge.
local function finish(grid, y, ruled, inner)
  grid[y].fill = grid[y].fill or "sidebar:bar"
  if ruled then
    pad_to(grid[y], inner)
    put(grid, y, inner, 1, { { text = M.rule, hl = "sidebar:rule" } })
  end
end

-- A row's own fill is its group's `:bg` twin up to the rule, laid before the text's marks so their
-- foregrounds keep it; the band's colour, and a fill with no twin, is the whole width, rule too.
local function mark(buf, y, row, ruled, inner)
  local own = row.fill ~= "sidebar:bar" and neotmux.hl.get(row.fill .. ":bg")
  if own and ruled then
    neotmux.buf.set_extmark(buf, ns, y, 0, { line_hl_group = "sidebar:bar" })
    neotmux.buf.set_extmark(buf, ns, y, 0, { end_col = inner, hl_group = row.fill .. ":bg" })
  else
    neotmux.buf.set_extmark(buf, ns, y, 0, { line_hl_group = row.fill })
  end
  for _, m in ipairs(row.marks) do
    neotmux.buf.set_extmark(
      buf,
      ns,
      y,
      m.col,
      { end_col = m.end_col, hl_group = m.hl, bold = m.bold }
    )
  end
end

-- The whole band as lines, row for row, written into `buf`. Answers where each section's rows
-- landed, for `patch`; nil when folded, which is never patched.
function M.render(band, buf)
  local grid = {}
  for y = 1, band.h do
    grid[y] = { text = "", cols = 0, marks = {} }
  end
  local rail = not M.open

  -- A section with nothing to show takes no room and draws no header, unless it is `always`.
  local live = {}
  local layout = M.open and { sections = {} } or nil
  for at, section in ipairs(M.sections) do
    local model = guarded(section, at, "build", section.build, M)
    if type(model) ~= "table" or type(model.rows) ~= "table" then
      layout = nil
    else
      if layout then
        layout.sections[at] = { section = section, at = at, model = model, y_of = {} }
      end
      if rail and section.rail then
        local rows = guarded(section, at, "rail", section.rail, M)
        model = { rows = type(rows) == "table" and rows or {} }
      end
      if #model.rows > 0 or section.always then
        live[#live + 1] = { section = section, at = at, model = model }
      end
    end
  end
  for i, it in ipairs(live) do
    it.heading = heading(it.section, i == 1, rail)
  end

  -- The width the content wants, asked for once it is known. `resize_to` lands on the next
  -- frame, which is why this is a comparison and not a call every render.
  local want, widest = width_for(live)
  if not M.open then
    want = M.widths.closed
  end
  if M.band and want ~= band.w then
    M.band:resize_to(want)
  end

  local ruled = band.w > 1
  local inner = ruled and band.w - 1 or band.w

  -- The bottom row is the fold's: `«` in its corner open, `»` in the middle of the rail.
  local height = band.h
  if M.band and M.folds and height > 1 then
    height = height - 1
    local glyph = rail and M.folds.closed or M.folds.open
    local x = rail and math.floor((inner - 1) / 2) or inner - 1
    pad_to(grid[height + 1], x)
    put(grid, height + 1, x, 1, { { text = glyph, hl = "sidebar:overlay" } })
  end

  local heights = heights_of(live, height)
  local y = 1
  for i, it in ipairs(live) do
    local body = heights[i] - it.heading
    -- A header with no room for a row under it is not drawn, unless there are no rows to want.
    local room = heights[i] >= it.heading and (body >= 1 or #it.model.rows == 0)
    local top = y
    if i > 1 and room then
      put(grid, top, 0, inner, { { text = string.rep("\u{2500}", inner), hl = "sidebar:rule" } })
      top = top + 1
    end
    if it.section.title and not rail and room then
      put(grid, top, 0, inner, header(it.section, inner))
      top = top + 2
    end
    if body >= 1 then
      local fit = (not rail or not it.section.rail) and it.section.fit or default_fit
      local rows = guarded(it.section, it.at, "fit", fit, it.model, body, M)
      if type(rows) == "table" then
        local index = {}
        for k, runs in ipairs(it.model.rows) do
          index[runs] = k
        end
        for k = 1, math.min(#rows, body) do
          put(grid, top + k - 1, 0, inner, rows[k])
          if layout and index[rows[k]] then
            layout.sections[it.at].y_of[index[rows[k]]] = top + k - 1
          end
        end
      end
    end
    y = y + heights[i]
  end

  local lines = {}
  for row = 1, band.h do
    finish(grid, row, ruled, inner)
    lines[row] = grid[row].text
  end
  neotmux.buf.set_lines(buf, 0, -1, lines)
  neotmux.buf.clear_namespace(buf, ns, 0, -1)
  for row = 1, band.h do
    mark(buf, row - 1, grid[row], ruled, inner)
  end
  if layout then
    layout.want, layout.widest, layout.inner, layout.ruled = want, widest, inner, ruled
    layout.count = #M.sections
  end
  return layout
end

-- When to render: the model is stale when something it shows changed. Events say most of it; a
-- token of your own that shows something else must call `dirty()` when that changes. A count,
-- not a flag, so every session's tick sees a change however many ticks ran since.
local structure = 0
function M.dirty()
  structure = structure + 1
end

-- What only rewrites rows: a pane's facts moving. `judged[pane]` is the count when its last did.
local changes = 0
local judged = {}

-- Per pane, the facts its rows depend on: `state|command|agent`.
local facts = {}

local function judge(pane)
  local now = seen_state.of(pane, require("neotmux.state").of(pane))
  local info = neotmux.pane.info(pane)
  local fact = tostring(now)
    .. "|"
    .. tostring(info and info.command)
    .. "|"
    .. tostring(require("neotmux.detect").agent(pane))
  if facts[pane] ~= fact then
    facts[pane] = fact
    changes = changes + 1
    judged[pane] = changes
  end
end

g.on_event("pane_changed", function(e)
  judge(e.pane)
end)
g.on_event("pane_info_changed", function(e)
  judge(e.pane)
end)
g.on_event("pane_closed", function(e)
  facts[e.pane], judged[e.pane] = nil, nil
  M.dirty()
end)
for _, name in ipairs({
  "pane_opened",
  "focus_changed",
  "resized",
  "window_opened",
  "window_closed",
  "window_changed",
  "window_renamed",
  "session_created",
}) do
  g.on_event(name, M.dirty)
end

-- What changes with time and no output besides a pane's state, which `neotmux.seen` follows: a
-- branch. Every session's directory asked again once its refresh has passed.
--
-- A timer belongs to the first session to run a frame after it is armed and ends with it, so
-- any session closing arms a fresh one for whichever frames next.
local clock
local function arm()
  clock = g.timer.every(1000, function()
    local spaces = package.loaded["plugin.herdr.sidebar.spaces"]
    if spaces then
      spaces.poke()
    end
  end)
end
arm()

-- Per session a client is at, the buffer its band shows and what it was last rendered against.
local bufs, last = {}, {}

g.on_event("session_closed", function(e)
  M.dirty()
  if bufs[e.session] then
    neotmux.buf.delete(bufs[e.session])
  end
  bufs[e.session], last[e.session] = nil, nil
  if clock then
    clock:stop()
  end
  arm()
end)
g.on_event("session_renamed", function(e)
  M.dirty()
  bufs[e.session], bufs[e.from] = bufs[e.from], nil
  last[e.session], last[e.from] = last[e.from], nil
end)

local function same(a, b)
  if a.fill ~= b.fill or a.blank ~= b.blank or #a ~= #b then
    return false
  end
  for k, run in ipairs(a) do
    local other = b[k]
    if
      run.text ~= other.text
      or run.hl ~= other.hl
      or run.bold ~= other.bold
      or run.shrink ~= other.shrink
    then
      return false
    end
  end
  return true
end

-- The rows that changed since `was` rewritten in place, for a change that moved no row: each
-- section's `patch(sidebar, model, change)` answers `{ { at = model row, runs = ... }, ... }`
-- for the rows it would build differently, or nil when that is not a row for row change. False
-- when the band has to be laid out again instead.
local function patch(was, buf, stale)
  local layout = was.layout
  if layout == nil or layout.count ~= #M.sections or layout.want ~= was.w then
    return false
  end
  local panes = {}
  for pane, n in pairs(judged) do
    if n > was.changes then
      panes[pane] = true
    end
  end
  if was.seen ~= seen_state.generation then
    -- A pane that moved it and is gone since is a pane closing, which is a new layout.
    local found = false
    for pane, n in pairs(seen_state.moved) do
      if n > was.seen then
        panes[pane], found = true, true
      end
    end
    if not found then
      return false
    end
  end
  local change = { panes = panes, aged = stale }
  local edits = {}
  for at, section in ipairs(M.sections) do
    local rec = layout.sections[at]
    if rec == nil or rec.section ~= section or type(section.patch) ~= "function" then
      return false
    end
    local got = guarded(section, at, "patch", section.patch, M, rec.model, change)
    if type(got) ~= "table" then
      return false
    end
    for _, edit in ipairs(got) do
      local old = rec.model.rows[edit.at]
      if old == nil then
        return false
      end
      if not same(old, edit.runs) then
        edits[#edits + 1] = { rec = rec, at = edit.at, old = old, runs = edit.runs }
      end
    end
  end
  -- A row not on screen may be one a fit would now keep, a row of another width may be one
  -- the band would be resized for, and a row going blank or not is one a fit cuts first.
  local widest = layout.widest
  for _, edit in ipairs(edits) do
    if not edit.rec.y_of[edit.at] or edit.old.blank ~= edit.runs.blank then
      return false
    end
    local wide, before = segments.width(edit.runs), segments.width(edit.old)
    widest = math.max(widest, wide)
    local shrinks = wide < before and before >= layout.widest
    if width_at(widest) ~= layout.want or (shrinks and width_at(wide) ~= layout.want) then
      return false
    end
  end
  layout.widest = widest
  for _, edit in ipairs(edits) do
    local y = edit.rec.y_of[edit.at]
    local grid = { { text = "", cols = 0, marks = {} } }
    put(grid, 1, 0, layout.inner, edit.runs)
    finish(grid, 1, layout.ruled, layout.inner)
    neotmux.buf.set_lines(buf, y - 1, y, { grid[1].text })
    mark(buf, y - 1, grid[1], layout.ruled, layout.inner)
    edit.rec.model.rows[edit.at] = edit.runs
  end
  return true
end

-- Each session a client is at renders into its own buffer, laid out for its own band, in its own
-- tick; a session nobody is at renders nothing. A change that is only some pane's state rewrites
-- the rows showing it, when `patch` can.
g.on_tick(function(ctx)
  local band = ctx:reserved()
  -- Nil while the terminal is too narrow to spare the columns. The panes have them back, so
  -- there is nowhere to draw and nothing to draw over.
  if not band then
    return
  end
  local who = neotmux.session.current()
  if who == nil or neotmux.session.attached(who) == 0 then
    return
  end
  -- This file may have begun loading first, and so tick before `seen` does.
  seen_state.update()
  if not bufs[who] then
    bufs[who] = g.buf.create()
    M.band:set_buf(bufs[who])
  end
  -- What the rows are laid out against, besides the model.
  local spaces = package.loaded["plugin.herdr.sidebar.spaces"]
  local agents = package.loaded["plugin.herdr.sidebar.agents"]
  local spaces_cursor = spaces and spaces.cursor
  local agents_cursor = agents and agents.cursor
  local sort = agents and agents.sort
  local was = last[who]
  local laid = was ~= nil
    and was.structure == structure
    and was.w == band.w
    and was.h == band.h
    and was.open == M.open
    and was.spaces_cursor == spaces_cursor
    and was.agents_cursor == agents_cursor
    and was.sort == sort
  local stale = laid and was.stale_at ~= nil and agents.clock() >= was.stale_at
  if laid and not stale and was.changes == changes and was.seen == seen_state.generation then
    return
  end
  if laid and patch(was, bufs[who], stale) then
    was.changes, was.seen = changes, seen_state.generation
    was.stale_at = agents and agents.stale_at
    return
  end
  local layout = M.render(band, bufs[who])
  last[who] = {
    structure = structure,
    changes = changes,
    seen = seen_state.generation,
    w = band.w,
    h = band.h,
    open = M.open,
    spaces_cursor = spaces_cursor,
    agents_cursor = agents_cursor,
    sort = sort,
    stale_at = agents and agents.stale_at,
    layout = layout,
  }
end)

-- Last, so a section's own file can already find everything above in the module it is handed.
-- herdr's two halves. The tree of the session you are in is `plugin.herdr.sidebar.sessions`, kept
-- for anyone who wants it:
-- `table.insert(sidebar.sections, 1, require('plugin.herdr.sidebar.sessions'))`.
M.sections = {
  require("plugin.herdr.sidebar.spaces"),
  require("plugin.herdr.sidebar.agents"),
}

-- `C-b w`, herdr's navigate: a cursor over the spaces, and `C-b a` the same over the agents.
-- Bound here rather than in the sections, for the reason `b` is: the prefix is the standard
-- plugin and this one is optional.
do
  local bind = require("neotmux.bind")
  bind.prefix(
    "w",
    require("plugin.herdr.sidebar.spaces").navigate,
    "move a cursor over the sidebar's sessions"
  )
  bind.prefix(
    "a",
    require("plugin.herdr.sidebar.agents").navigate,
    "move a cursor over the sidebar's agents"
  )
end

return M

-- A sidebar down the left: a stack of sections, each a table saying what rows it has and how to
-- cut them to a height. The shipped ones are herdr's: every session, `plugin.sidebar.spaces`,
-- and under it every agent in every session, `plugin.sidebar.agents`.
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
--   table.insert(require('plugin.sidebar').sections, {
--     title = 'mine',   -- a header above it; nil for none
--     share = 0.3,      -- the most of the band it takes when there is not room for everybody
--     always = true,    -- headed even with nothing to show
--     build = function(sidebar) return { rows = { runs, ... }, anchor = 1 } end,
--     fit = function(model, height, sidebar) return { runs, ... } end,   -- optional
--     rail = function(sidebar) return { runs, ... } end,   -- optional: what it is folded
--   })
--
-- A row is a list of runs, the shape `status.segments.draw` takes, and a row with a `fill` has the
-- band's width behind it painted that colour. `build` is handed this module
-- rather than requiring it, because the shipped sections are required from here while this file
-- is still loading. Without a `fit`, a section is cut by `sidebar.scroll`.
--
-- Nothing here is a host feature. `neotmux.reserve('left', n)` takes the band,
-- `handle:resize_to(n)` widens and folds it, and `ctx:reserved()` answers where it landed.
local segments = require("status.segments")

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

-- Catppuccin Mocha, the palette the status bar was drawn from. Every section reads these, and
-- every field is read on the tick, so assigning to one from your own `init.lua` takes effect.
M.colors = {
  -- The band everything sits on: nil is the terminal's own background, which is herdr's.
  bar = nil,
  text = "#cdd6f4",
  subtext = "#a6adc8", -- subtext0: a name you are not at
  overlay = "#6c7086", -- overlay0: headers, and what is detail rather than name
  branch = "#cba6f7", -- mauve: the branch of the session you are in
  -- The rows you are at: the session you are in and the agent you are focused on.
  here = "#1e1e2e", -- base
  header = "#6c7086",
  session = "#9399b2", -- overlay2: a session you are not in
  current_session = "#f9e2af", -- yellow, so a session reads as a different kind of row
  window = "#cdd6f4",
  current_window = "#cba6f7", -- mauve, the status bar's accent for the window being shown
  pane = "#6c7086", -- overlay0: a pane in a window you are not looking at
  focused_pane = "#a6e3a1", -- green
  -- The second row of an entry, and anything else that is detail rather than name.
  detail = "#585b70", -- surface2
  rule = "#1e1e2e", -- base: the band's right edge and the line between sections
  -- One per state, so the icon carries its meaning in the colour as well as the shape.
  -- herdr's, so a herdr user reads them without learning them again.
  working = "#f9e2af", -- yellow
  blocked = "#f38ba8", -- red: the one that wants you
  done = "#94e2d5", -- teal: finished while you were not looking (`neotmux.seen`)
  idle = "#a6e3a1", -- green
  unknown = "#6c7086", -- a session with no agent in it, which has no state to say
  -- The navigate cursor (`C-b w`).
  selection = "#313244", -- surface0
}

-- One per state, herdr's `dots`: the colour says which, the shape only whether anything is
-- going on. herdr's `symbols` are one assignment away:
--
--   require('plugin.sidebar').icons = { working = '◐', blocked = '×', done = '✓', idle = '○' }
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

M.band = neotmux.reserve("left", M.widths.min)

-- Which of the two the band is in. Folded, a section with a `rail` draws that, and one without is
-- its open rows cut to the rail's width.
M.open = true

-- Where the band is cut between two sections, as a part of its height; herdr cuts at half. Nil
-- shares it out by what each section has, which is also what three or more sections get.
M.split = 0.5

-- The bottom row's corner: what `C-b b` would do. Nil gives the row back to the sections.
M.folds = { open = "\u{ab}", closed = "\u{bb}" } -- «, »

-- One row: every token in `names` that has something to say, in order, separated by a space.
-- A token answers its text, its colour and whether it is bold. One that answers nil or an empty
-- string is left out, so a row whose tokens are all empty is no row.
function M.row(tokens, names, it)
  local runs = {}
  for _, name in ipairs(names) do
    local token = tokens[name]
    if token then
      local text, colour, bold = token(it)
      if text and text ~= "" then
        if #runs > 0 then
          runs[#runs + 1] = { text = " ", bg = M.colors.bar }
        end
        runs[#runs + 1] =
          { text = text, fg = colour or M.colors.detail, bg = M.colors.bar, bold = bold }
      end
    end
  end
  return runs
end

-- What `scroll` puts where it cut rows: `n` counts the row the mark itself stands in, so the
-- number is how many rows are not on screen on that side.
local function arrow(glyph)
  return function(n)
    return { { text = glyph .. " " .. n, fg = M.colors.detail, bg = M.colors.bar } }
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
    local which = "plugin.sidebar: section '" .. tostring(section.title or at) .. "' " .. what
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
  local style = { fg = M.colors.header, bg = M.colors.bar, bold = true }
  return {
    { text = left, fg = style.fg, bg = style.bg, bold = true },
    { text = string.rep(" ", gap), bg = style.bg },
    { text = right, fg = style.fg, bg = style.bg, bold = true },
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

local function draw_row(ctx, x, y, width, runs)
  if runs.fill then
    ctx:rect(nil, x, y, width, 1, { bg = runs.fill })
    for _, run in ipairs(runs) do
      run.bg = run.bg or runs.fill
    end
  end
  segments.draw(ctx, x, y, width, runs)
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

-- How wide the band wants to be: the widest row any section has, between the two bounds.
local function width_for(live)
  local widest = 0
  for _, it in ipairs(live) do
    for _, runs in ipairs(it.model.rows) do
      widest = math.max(widest, segments.width(runs))
    end
  end
  -- One for a column of air, one for the rule.
  return math.max(M.widths.min, math.min(M.widths.max, widest + 2))
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
-- knowledge goes in. The defaults table and not only `package.loaded`: an `init.lua` that
-- requires this file to change it runs before the shipped layer's, so the prefix is not loaded
-- yet, and a plain probe would leave `C-b b` unbound — `plugin.palette` binds `:` this way for
-- the same reason.
local function prefix()
  if package.loaded["plugin.prefix"] or require("neotmux.defaults").prefix then
    return require("plugin.prefix")
  end
end

if prefix() then
  prefix().commands["b"] = M.toggle
end

neotmux.on_tick(function(ctx)
  local band = ctx:reserved()
  -- Nil while the terminal is too narrow to spare the columns. The panes have them back, so
  -- there is nowhere to draw and nothing to draw over.
  if not band then
    return
  end
  local rail = not M.open

  -- A section with nothing to show takes no room and draws no header, unless it is `always`.
  local live = {}
  for at, section in ipairs(M.sections) do
    local model = guarded(section, at, "build", section.build, M)
    if type(model) == "table" and type(model.rows) == "table" then
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
  -- frame, which is why this is a comparison and not a call every tick.
  if M.band then
    local want = M.open and width_for(live) or M.widths.closed
    if want ~= band.w then
      M.band:resize_to(want)
    end
  end

  ctx:rect(nil, band.x, band.y, band.w, band.h, { bg = M.colors.bar })

  local ruled = band.w > 1
  local inner = ruled and band.w - 1 or band.w
  if ruled then
    for row = band.y, band.y + band.h - 1 do
      ctx:text(nil, band.x + inner, row, M.rule, { fg = M.colors.rule, bg = M.colors.bar })
    end
  end

  -- The bottom row is the fold's: `«` in its corner open, `»` in the middle of the rail.
  local height = band.h
  if M.band and M.folds and height > 1 then
    height = height - 1
    local glyph = rail and M.folds.closed or M.folds.open
    local x = rail and band.x + math.floor((inner - 1) / 2) or band.x + inner - 1
    ctx:text(nil, x, band.y + height, glyph, { fg = M.colors.overlay, bg = M.colors.bar })
  end

  local heights = heights_of(live, height)
  local y = band.y
  for i, it in ipairs(live) do
    local body = heights[i] - it.heading
    -- A header with no room for a row under it is not drawn, unless there are no rows to want.
    local room = heights[i] >= it.heading and (body >= 1 or #it.model.rows == 0)
    local top = y
    if i > 1 and room then
      local line = string.rep("\u{2500}", inner)
      ctx:text(nil, band.x, top, line, { fg = M.colors.rule, bg = M.colors.bar })
      top = top + 1
    end
    if it.section.title and not rail and room then
      segments.draw(ctx, band.x, top, inner, header(it.section, inner))
      top = top + 2
    end
    if body >= 1 then
      local fit = (not rail or not it.section.rail) and it.section.fit or default_fit
      local rows = guarded(it.section, it.at, "fit", fit, it.model, body, M)
      if type(rows) == "table" then
        for k = 1, math.min(#rows, body) do
          draw_row(ctx, band.x, top + k - 1, inner, rows[k])
        end
      end
    end
    y = y + heights[i]
  end
end)

-- Last, so a section's own file can already find everything above in the module it is handed.
-- herdr's two halves. The tree of the session you are in is `plugin.sidebar.sessions`, kept for
-- anyone who wants it: `table.insert(sidebar.sections, 1, require('plugin.sidebar.sessions'))`.
M.sections = {
  require("plugin.sidebar.spaces"),
  require("plugin.sidebar.agents"),
}

-- `C-b w`, herdr's navigate: a cursor over the spaces. Bound here rather than in the section, for
-- the reason `b` is: the prefix is the standard plugin and this one is optional.
if prefix() then
  prefix().commands["w"] = require("plugin.sidebar.spaces").navigate
end

return M

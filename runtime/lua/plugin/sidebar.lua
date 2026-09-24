-- A sidebar down the left: a stack of sections, each a table saying what rows it has and how to
-- cut them to a height. The shipped one is the session tree, `plugin.sidebar.sessions`.
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
--     build = function(sidebar) return { rows = { runs, ... }, anchor = 1 } end,
--     fit = function(model, height, sidebar) return { runs, ... } end,   -- optional
--   })
--
-- A row is a list of runs, the shape `status.segments.draw` takes. `build` is handed this module
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
  min = 27,
  max = 36,
  -- What is left when it is folded: one column, so the markers still say how many sessions and
  -- panes there are and which of them you are in. A band of nothing would be a band you have to
  -- remember is there.
  closed = 1,
}

-- Catppuccin Mocha, the palette the status bar was drawn from. Every section reads these, and
-- every field is read on the tick, so assigning to one from your own `init.lua` takes effect.
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
  rule = "#45475a", -- surface1: a section's header
  -- One per state, so the icon carries its meaning in the colour as well as the shape.
  working = "#a6e3a1", -- green
  blocked = "#f9e2af", -- yellow: the one that wants you
  idle = "#585b70", -- surface2
}

M.band = neotmux.reserve("left", M.widths.min)

-- Which of the two the band is in. The rows are the same either way: `segments.draw` cuts a run
-- that does not fit, so the rail is the open sidebar with everything but the first column
-- clipped off, and there is no second way of drawing this to keep in step with the first.
M.open = true

-- One row: every token in `names` that has something to say, in order, separated by a space.
-- A token that answers nil or an empty string is left out, so a row whose tokens are all empty
-- is no row.
function M.row(tokens, names, it)
  local runs = {}
  for _, name in ipairs(names) do
    local token = tokens[name]
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

-- The default fit: every row if they fit, and otherwise a window of them centred on `anchor`
-- and clamped to the ends, so the row that matters is always one of the ones that stayed.
function M.scroll(model, height)
  local rows = model.rows
  if height <= 0 then
    return {}
  end
  if #rows <= height then
    return rows
  end
  local anchor = model.anchor or 1
  local first = math.min(math.max(anchor - math.floor(height / 2), 1), #rows - height + 1)
  local out = {}
  for at = first, first + height - 1 do
    out[#out + 1] = rows[at]
  end
  return out
end

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
    wants[i] = #it.model.rows + (it.titled and 1 or 0)
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

local function header(title, width)
  local text = "\u{2500} " .. title .. " "
  local rest = width - segments.columns(text)
  if rest > 0 then
    text = text .. string.rep("\u{2500}", rest)
  end
  return { { text = text, fg = M.colors.rule, bg = M.colors.bar } }
end

-- How wide the band wants to be: the widest row any section has, between the two bounds.
local function width_for(live)
  local widest = 0
  for _, it in ipairs(live) do
    for _, runs in ipairs(it.model.rows) do
      widest = math.max(widest, segments.width(runs))
    end
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

  -- A section with nothing to show takes no room and draws no header. The first section that
  -- does show something draws no header either: the top of the band says where it starts.
  local live = {}
  for at, section in ipairs(M.sections) do
    local model = guarded(section, at, "build", section.build, M)
    if type(model) == "table" and type(model.rows) == "table" and #model.rows > 0 then
      live[#live + 1] = {
        section = section,
        at = at,
        model = model,
        titled = #live > 0 and section.title ~= nil,
      }
    end
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

  local heights = share_out(live, band.h)
  local y = band.y
  for i, it in ipairs(live) do
    local body = heights[i] - (it.titled and 1 or 0)
    if body >= 1 then
      local fit = it.section.fit or default_fit
      local rows = guarded(it.section, it.at, "fit", fit, it.model, body, M)
      if type(rows) == "table" then
        local top = y
        if it.titled then
          segments.draw(ctx, band.x, top, band.w, header(it.section.title, band.w))
          top = top + 1
        end
        for k = 1, math.min(#rows, body) do
          segments.draw(ctx, band.x, top + k - 1, band.w, rows[k])
        end
      end
    end
    y = y + heights[i]
  end
end)

-- Last, so a section's own file can already find everything above in the module it is handed.
M.sections = {
  require("plugin.sidebar.sessions"),
}

return M

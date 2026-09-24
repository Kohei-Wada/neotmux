-- The command palette: every command a plugin has ever registered through `neotmux.command`,
-- fuzzy-filtered as you type, run with Enter.
--
--   C-b :       open it
--   type        narrow to commands whose name holds the query's characters in order
--   C-j C-k     the cursor, one row — plain `j`/`k` are query text while filtering
--   Enter       run the command under the cursor and close
--   Esc q       close
--
-- A composition on `neotmux.list`, the same shape `plugin.sessions` is: the float, the cursor,
-- the caret and the input line are that module's. What is here is what makes this list a
-- palette rather than any other one — the fuzzy scorer, what a row says, and what Enter does.
-- It lists commands, not keys: `plugin.prefix` registers its own verbs this way (see its own
-- file), and any other plugin that calls `neotmux.command.add` gets a palette entry for free,
-- without this file knowing it exists.
local list = require("neotmux.list")

local M = {}

M.colors = {
  name = "#cdd6f4",
  desc = "#a6adc8", -- subtext0: quieter than the name
  source = "#585b70", -- surface2: quieter still
}

M.width = 60
M.rows = 12

-- A query matches `name` when its characters appear in `name`, in order, gaps allowed —
-- `fb` matches `foobar` — and answers where the first one landed and how far the last one
-- was from it, the two numbers a match is ranked by: an earlier first match sorts before a
-- later one, and among equal starts, a tighter run of the rest sorts first. `nil` is no
-- match. Case-insensitive, so `NEW` finds `window-new`. An empty query matches at the front
-- with nothing to spread, safe on its own terms rather than only because `ranked` never asks
-- it the question.
local function fuzzy(name, query)
  if query == "" then
    return 1, 0
  end
  local hay = name:lower()
  local needle = query:lower()
  local first, last
  local at = 1
  for i = 1, #needle do
    local found = hay:find(needle:sub(i, i), at, true)
    if not found then
      return nil
    end
    first = first or found
    last = found
    at = found + 1
  end
  return first, last - first
end

-- Ranked by `fuzzy`'s two numbers, then by name for anything still tied. Every non-match is
-- dropped here, so `filter`, below, has nothing left to reject.
local function ranked(items, query)
  if query == "" then
    return items
  end
  local hits = {}
  for _, item in ipairs(items) do
    local first, spread = fuzzy(item.name, query)
    if first then
      hits[#hits + 1] = { item = item, first = first, spread = spread }
    end
  end
  table.sort(hits, function(a, b)
    if a.first ~= b.first then
      return a.first < b.first
    end
    if a.spread ~= b.spread then
      return a.spread < b.spread
    end
    return a.item.name < b.item.name
  end)
  local out = {}
  for at, hit in ipairs(hits) do
    out[at] = hit.item
  end
  return out
end

-- One row: the name, its description in a quieter colour, and where the row is wide enough
-- to hold it, the source that registered it, quieter still and pushed to the right — the same
-- layout `plugin.sessions` draws its own row in.
local function row(item)
  local desc = item.desc and ("  " .. item.desc) or ""
  local runs = {
    { text = item.name, fg = M.colors.name },
    { text = desc, fg = M.colors.desc },
  }
  local source = item.source or ""
  local used = neotmux.width(item.name) + neotmux.width(desc)
  local room = M.width - 2 - 2 - used - neotmux.width(source) - 1
  if source ~= "" and room > 1 then
    runs[#runs + 1] = { text = string.rep(" ", room) }
    runs[#runs + 1] = { text = source, fg = M.colors.source }
  end
  return runs
end

function M.open()
  -- Assigned once `list.open` has returned a handle, and read by `items` on every later
  -- refresh: the same upvalue trick `plugin.sessions` uses for state that only exists once
  -- the list is up. `list.open` calls `items` once on the way in, before `handle` exists,
  -- which `and` guards against.
  local handle

  handle = list.open({
    title = "commands",
    mode = "palette",
    width = M.width,
    rows = M.rows,
    filter = function()
      return true
    end,
    items = function()
      local query = handle and handle:query() or ""
      return ranked(neotmux.command.list(), query)
    end,
    row = row,
    done = function(item, done_handle)
      local ok, why, lines = neotmux.command.run(item.name)
      done_handle:close()
      if not ok then
        neotmux.notify("error", why)
      elseif lines then
        for _, line in ipairs(lines) do
          neotmux.notify("info", line)
        end
      end
    end,
  })
end

-- The defaults table and not just `package.loaded`, because a user who rebinds the key above
-- has to `require` this file from their own `init.lua`, which runs before the shipped layer's
-- — so the prefix is not loaded yet and a plain probe would silently leave `C-b :` unbound.
-- Loading it here is what it was about to do anyway. The same shape `plugin.copy` binds `[`
-- with.
if package.loaded["plugin.prefix"] or require("neotmux.defaults").prefix then
  require("plugin.prefix").commands[":"] = function()
    M.open()
  end
end

return M

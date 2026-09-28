-- The command palette: every command a plugin has ever registered through `neotmux.command`,
-- fuzzy-filtered as you type, run with Enter.
--
--   C-b :       open it
--   type        narrow to commands whose name holds the query's characters in order
--   C-j C-k     the cursor, one row — plain `j`/`k` are query text, `q` included
--   Enter       run the command under the cursor and close
--   Esc         close
--
-- A composition on `neotmux.list`, the same shape `plugin.tmux.sessions` is: the float, the cursor,
-- the caret and the input line are that module's. What is here is what makes this list a
-- palette rather than any other one — the fuzzy scorer, what a row says, and what Enter does.
-- It lists commands, not keys: `plugin.tmux.prefix` registers its own verbs this way (see its own
-- file), and any other plugin that calls `neotmux.command.add` gets a palette entry for free,
-- without this file knowing it exists.
local list = require("neotmux.list")
local style = require("neotmux.style")
-- Required for its tick: `#` shows `done`, and done is only known to what watched it happen.
require("neotmux.seen")

local M = {}

style.default("palette", {
  name = "text",
  match = "blue", -- the letters you actually typed, lit in the name
  desc = "subtext0", -- quieter than the name
  source = "surface2", -- quieter still
})

-- Wide and shallow, hanging from the top: what you are doing here is reading a line you typed
-- and the handful of things it left, not surveying a list. A width of one or less is a share
-- of the pane area, so this follows the terminal rather than sitting at some column count
-- somebody picked once.
M.width = 0.8
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
    return 1, 0, {}
  end
  local hay = name:lower()
  local needle = query:lower()
  local first, last
  local at = 1
  -- Where each character of the query landed, so the row can show its work: the letters you
  -- typed are lit in the name, which is how you tell `window-new` from `window-next` when the
  -- query is `wn` and both of them matched.
  local where = {}
  for i = 1, #needle do
    local found = hay:find(needle:sub(i, i), at, true)
    if not found then
      return nil
    end
    first = first or found
    last = found
    where[#where + 1] = found
    at = found + 1
  end
  return first, last - first, where
end

-- Ranked by `fuzzy`'s two numbers, then by name for anything still tied. Every non-match is
-- dropped here: `list.open` below gets `input = true` and no `filter`, so this is the only
-- narrowing a query does.
local function ranked(items, query)
  if query == "" then
    return items
  end
  local hits = {}
  for _, item in ipairs(items) do
    local first, spread, where = fuzzy(item.name, query)
    if first then
      -- A copy, not the registry's own table: `lit` belongs to this query and nothing else
      -- should find it there on the next one.
      local shown =
        { name = item.name, desc = item.desc, source = item.source, lit = where, item = item }
      hits[#hits + 1] = { item = shown, first = first, spread = spread }
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
-- layout `plugin.tmux.sessions` draws its own row in.
-- The name, split so the characters the query matched are in their own runs. A run per
-- stretch rather than per character: adjacent letters read as a word that way, and the drawing
-- does less work.
local function name_runs(c, item)
  local lit = item.lit
  if not lit or #lit == 0 then
    return { { text = item.name, fg = c.name } }
  end
  local runs = {}
  local at = 1
  local on = {}
  for _, where in ipairs(lit) do
    on[where] = true
  end
  while at <= #item.name do
    local start = at
    -- `on` holds true or nothing, and nothing is not false: comparing them without settling
    -- that first ends every stretch after one character.
    local matched = on[at] or false
    while at <= #item.name and (on[at] or false) == matched do
      at = at + 1
    end
    runs[#runs + 1] = {
      text = item.name:sub(start, at - 1),
      fg = matched and c.match or c.name,
    }
  end
  return runs
end

local function row(item, _chosen, width)
  local c = style.get("palette")
  local desc = item.desc and ("  " .. item.desc) or ""
  local runs = name_runs(c, item)
  runs[#runs + 1] = { text = desc, fg = c.desc }
  local source = item.source or ""
  local used = neotmux.width(item.name) + neotmux.width(desc)
  local room = (width or 60) - 2 - used - neotmux.width(source) - 1
  if source ~= "" and room > 1 then
    runs[#runs + 1] = { text = string.rep(" ", room) }
    runs[#runs + 1] = { text = source, fg = c.source }
  end
  return runs
end

-- Where the items come from. The query's first character picks one — `@` the sessions, `#` the
-- agents — and the one with no prefix takes everything else, the commands. A plugin adds its own:
--
--   table.insert(require('plugin.palette').providers, 1, {
--     prefix = '%', name = 'fruit',
--     items = function() return { { name = 'apple', desc = 'red' } } end,
--     run = function(item) ... end,
--   })
--
-- An item is `{ name =, desc =, source = }` and anything else `run` needs; every provider is
-- ranked by the same fuzzy match on `name`.
local function run_command(item)
  local ok, why, lines = neotmux.command.run(item.name)
  if not ok then
    neotmux.notify("error", why)
  elseif lines then
    for _, line in ipairs(lines) do
      neotmux.notify("info", line)
    end
  end
end

M.providers = {
  {
    prefix = "@",
    name = "sessions",
    items = function()
      local out = {}
      for _, name in ipairs(neotmux.session.list()) do
        out[#out + 1] = { name = name, source = "session" }
      end
      return out
    end,
    run = function(item)
      if item.name ~= neotmux.session.current() then
        neotmux.session.switch(item.name)
      end
    end,
  },
  {
    prefix = "#",
    name = "agents",
    items = function()
      local out = {}
      for _, it in ipairs(require("plugin.herdr.sidebar.agents").collect()) do
        local window = it.windows > 1 and (":" .. (it.window_name or it.window)) or ""
        -- The state is in the name, not a description beside it, so it is matched: typing
        -- `blocked` or `done` narrows to the agents in it.
        out[#out + 1] = {
          name = it.session .. window .. " " .. it.agent .. " " .. it.state,
          source = "agent",
          agent = it,
        }
      end
      return out
    end,
    run = function(item)
      require("plugin.herdr.sidebar.agents").focus(item.agent)
    end,
  },
  {
    prefix = "",
    name = "commands",
    items = function()
      return neotmux.command.list()
    end,
    run = run_command,
  },
}

-- A provider that raises is left out of that query and named once, for the reason a sidebar
-- section is: it will raise again on every keystroke after this one.
local complained = setmetatable({}, { __mode = "k" })

local function pick(query)
  local fallback
  for _, provider in ipairs(M.providers) do
    local prefix = provider.prefix or ""
    if prefix == "" then
      fallback = fallback or provider
    elseif query:sub(1, #prefix) == prefix then
      return provider, query:sub(#prefix + 1)
    end
  end
  return fallback, query
end

local function items_of(provider)
  local ok, items = pcall(provider.items)
  if ok and type(items) == "table" then
    return items
  end
  if not complained[provider] then
    complained[provider] = true
    local why = ok and "items did not answer a list" or tostring(items)
    neotmux.notify("error", "plugin.palette: provider '" .. tostring(provider.name) .. "': " .. why)
  end
  return {}
end

-- `prefix` is typed in already: `open('#')` is the agents, which is what `C-b g` is.
function M.open(prefix)
  -- Assigned once `list.open` has returned a handle, and read by `items` on every later
  -- refresh: the same upvalue trick `plugin.tmux.sessions` uses for state that only exists once
  -- the list is up. `list.open` calls `items` once on the way in, before `handle` exists,
  -- which `and` guards against.
  local handle
  local chosen

  handle = list.open({
    mode = "palette",
    width = M.width,
    rows = M.rows,
    place = "top",
    input = true,
    query = prefix,
    empty = "nothing matches",
    items = function()
      local provider, rest = pick(handle and handle:query() or prefix or "")
      chosen = provider
      if provider == nil then
        return {}
      end
      return ranked(items_of(provider), rest)
    end,
    row = row,
    done = function(item, done_handle)
      done_handle:close()
      local provider = chosen
      if provider and provider.run then
        local ok, err = pcall(provider.run, item.item or item)
        if not ok then
          neotmux.notify(
            "error",
            "plugin.palette: provider '" .. tostring(provider.name) .. "': " .. tostring(err)
          )
        end
      end
    end,
  })
end

local bind = require("neotmux.bind")
bind.prefix(":", function()
  M.open()
end, "open the command palette")
-- herdr's goto: every agent, and Enter goes to it.
bind.prefix("g", function()
  M.open("#")
end, "go to an agent, from the palette's list of them")

return M

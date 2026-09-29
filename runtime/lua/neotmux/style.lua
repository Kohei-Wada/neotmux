-- Which colour each part of each plugin is: a cascade over dotted selectors, in the theme's
-- names or in `#rrggbb`.
--
-- A convention, not a plugin: it registers nothing.
--
--   local style = require('neotmux.style')
--   style.default('status', { bar = 'mantle', accent = 'blue' })   -- a plugin, at load
--   style.set('float', { frame = 'crust' })                        -- the user, in init.lua
--   local c = style.get('status')                                  -- a plugin, when it draws
--
-- `get('float.list')` reads `*`, then `float`, then `float.list`, a later one winning key by key.
-- Every default comes before every set: the user's `init.lua` runs before the shipped layer
-- loads a single plugin, so if the later call won, every shipped default would override the
-- user. A plugin's `default` for a selector replaces its earlier one, so a reload does not stack
-- them; the user's `set`s merge.
local theme = require("neotmux.theme")

local M = {}

local defaults, sets = {}, {}
local cache, seen = {}, theme.generation

local function copy_into(into, from)
  for key, value in pairs(from) do
    into[key] = value
  end
end

function M.default(selector, parts)
  local own = {}
  copy_into(own, parts)
  defaults[selector] = own
  cache = {}
  M.publish()
end

function M.set(selector, parts)
  sets[selector] = sets[selector] or {}
  copy_into(sets[selector], parts)
  cache = {}
  M.publish()
end

-- `*`, then each dotted prefix of `selector`, ending with `selector` itself.
local function chain(selector)
  local out = { "*" }
  if selector == "*" then
    return out
  end
  local from = 1
  while true do
    local dot = selector:find(".", from, true)
    out[#out + 1] = dot and selector:sub(1, dot - 1) or selector
    if not dot then
      return out
    end
    from = dot + 1
  end
end

-- The merged, resolved colours for `selector`. Cached until anything changes, so reading it
-- where you draw, every frame, is a table lookup. The table is the cache's: read it, do not
-- write to it.
function M.get(selector)
  if seen ~= theme.generation then
    cache, seen = {}, theme.generation
  end
  local hit = cache[selector]
  if hit then
    return hit
  end
  local levels = chain(selector)
  local merged = {}
  for _, level in ipairs(levels) do
    copy_into(merged, defaults[level] or {})
  end
  for _, level in ipairs(levels) do
    copy_into(merged, sets[level] or {})
  end
  local resolved = {}
  for key, value in pairs(merged) do
    resolved[key] = theme.resolve(value)
  end
  cache[selector] = resolved
  return resolved
end

-- Every selector with a default or a set, published as `selector:part` highlight groups for
-- buffers to be coloured by, and each part as a background too, `selector:part:bg`. All of them
-- each time: a set on `float` changes `float.list`'s.
local function publish()
  if not (neotmux and neotmux.hl) then
    return
  end
  local selectors = {}
  for selector in pairs(defaults) do
    selectors[selector] = true
  end
  for selector in pairs(sets) do
    selectors[selector] = true
  end
  for selector in pairs(selectors) do
    for part, colour in pairs(M.get(selector)) do
      neotmux.hl.set(selector .. ":" .. part, { fg = colour })
      neotmux.hl.set(selector .. ":" .. part .. ":bg", { bg = colour })
    end
  end
end
M.publish = publish
theme._watch(publish)

return M

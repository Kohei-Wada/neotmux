-- The colours, by name: what every plugin's style is written in, so that swapping these
-- recolours all of them at once. Catppuccin Mocha, which every shipped colour is one of.
--
-- A convention, not a plugin: it registers nothing.
--
--   local theme = require('neotmux.theme')
--   theme.set({ blue = '#7aa2f7', base = '#1a1b26' })   -- some of them, or all
--   theme.use('nord')                                   -- a whole shipped theme, by name
local M = {
  colors = {},
  -- Moved by every `set`, so a cache of resolved colours knows to let go of them.
  generation = 0,
  -- `catppuccin-*` first, then the rest in the order they were added.
  names = {
    "catppuccin-latte",
    "catppuccin-frappe",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "tokyonight-night",
    "tokyonight-storm",
    "gruvbox-dark",
    "nord",
    "dracula",
    "rose-pine",
    "kanagawa",
    "onedark",
    "everforest-dark",
  },
  current = "catppuccin-mocha",
}

for name, hex in pairs(require("neotmux.themes.catppuccin-mocha")) do
  M.colors[name] = hex
end

-- Replaces the colours named in `t` and leaves the rest; a name of its own is added.
function M.set(t)
  for name, hex in pairs(t) do
    M.colors[name] = hex
  end
  M.generation = M.generation + 1
end

-- A whole shipped theme, by name — `theme.names`, `catppuccin-*` first. Raises for a name that
-- is not one of them.
function M.use(name)
  local ok, colors = pcall(require, "neotmux.themes." .. name)
  if not ok then
    error("unknown theme '" .. tostring(name) .. "'")
  end
  M.set(colors)
  M.current = name
end

-- A name's colour, a `#rrggbb` as itself, or nil for a name the theme does not have — which
-- a draw takes as the terminal's own colour, so a typo shows as a plain colour and not an error
-- on every tick.
function M.resolve(value)
  if type(value) ~= "string" then
    return nil
  end
  if value:sub(1, 1) == "#" then
    return value
  end
  return M.colors[value]
end

return M

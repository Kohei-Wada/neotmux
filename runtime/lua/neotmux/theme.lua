-- The colours, by name: what every plugin's style is written in, so that swapping these
-- recolours all of them at once. Catppuccin Mocha, which every shipped colour is one of.
--
-- A convention, not a plugin: it registers nothing.
--
--   local theme = require('neotmux.theme')
--   theme.set({ blue = '#7aa2f7', base = '#1a1b26' })   -- some of them, or all
local M = {
  colors = {
    rosewater = "#f5e0dc",
    flamingo = "#f2cdcd",
    pink = "#f5c2e7",
    mauve = "#cba6f7",
    red = "#f38ba8",
    maroon = "#eba0ac",
    peach = "#fab387",
    yellow = "#f9e2af",
    green = "#a6e3a1",
    teal = "#94e2d5",
    sky = "#89dceb",
    sapphire = "#74c7ec",
    blue = "#89b4fa",
    lavender = "#b4befe",
    text = "#cdd6f4",
    subtext1 = "#bac2de",
    subtext0 = "#a6adc8",
    overlay2 = "#9399b2",
    overlay1 = "#7f849c",
    overlay0 = "#6c7086",
    surface2 = "#585b70",
    surface1 = "#45475a",
    surface0 = "#313244",
    base = "#1e1e2e",
    mantle = "#181825",
    crust = "#11111b",
  },
  -- Moved by every `set`, so a cache of resolved colours knows to let go of them.
  generation = 0,
}

-- Replaces the colours named in `t` and leaves the rest; a name of its own is added.
function M.set(t)
  for name, hex in pairs(t) do
    M.colors[name] = hex
  end
  M.generation = M.generation + 1
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

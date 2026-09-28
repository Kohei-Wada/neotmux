-- The `theme` command: a list of every shipped theme, the current one marked, Enter to switch.
--
--   theme       (from the palette) pick a theme
--
-- Nothing is written to disk — the theme lasts until the config is reloaded or neotmux is
-- restarted. To keep it, add the line the command's own notification gives you to `init.lua`.
local list = require("neotmux.list")
local theme = require("neotmux.theme")
local style = require("neotmux.style")
local g = neotmux.group("plugin.theme")

style.default("theme", {
  name = "text",
  current = "green", -- the theme in use
})

local function row(name)
  local c = style.get("theme")
  local current = name == theme.current
  return {
    { text = current and "* " or "  ", fg = c.current },
    { text = name, fg = current and c.current or c.name },
  }
end

g.command.add("theme", function()
  list.open({
    title = "theme",
    mode = "theme",
    items = function()
      return theme.names
    end,
    row = row,
    done = function(name, handle)
      handle:close()
      theme.use(name)
      neotmux.notify(
        "info",
        "theme "
          .. name
          .. " — to keep it, add require('neotmux.theme').use('"
          .. name
          .. "') to init.lua"
      )
    end,
  })
end, { desc = "pick a theme; add require('neotmux.theme').use('<name>') to init.lua to keep it" })

return {}

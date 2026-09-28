-- What each plugin's ticks cost: a list of every source, most expensive first, with how many
-- ticks it has run, their time together, the mean and the longest. The palette's `costs`.
--
-- The numbers are `neotmux.costs()`, wall time around each source's whole tick since the
-- config loaded — its drawing, its reads of panes, everything it did that frame. They are the
-- thing to look at before deciding a plugin draws too often, rather than a guess.
local list = require("neotmux.list")
local g = neotmux.group("plugin.costs")

local M = {}

M.colors = { source = "#cba6f7", number = "#cdd6f4", label = "#6c7086" }

local function ms(value)
  return string.format("%8.2f", value)
end

local function row(it)
  local mean = it.ticks > 0 and it.total_ms / it.ticks or 0
  return {
    { text = string.format("%-24s", it.source), fg = M.colors.source },
    { text = string.format("%9d", it.ticks), fg = M.colors.number },
    { text = " ticks  ", fg = M.colors.label },
    { text = ms(it.total_ms), fg = M.colors.number },
    { text = " ms  mean", fg = M.colors.label },
    { text = ms(mean), fg = M.colors.number },
    { text = "  longest", fg = M.colors.label },
    { text = ms(it.longest_ms), fg = M.colors.number },
  }
end

function M.open()
  list.open({
    title = "costs",
    mode = "costs",
    width = 0.9,
    rows = 20,
    -- Read fresh on every tick, so the list counts up while it is open.
    items = function()
      return neotmux.costs()
    end,
    row = row,
    footer = function()
      return "every source's ticks since the config loaded  esc close"
    end,
    done = function(_, handle)
      handle:close()
    end,
  })
end

g.command.add("costs", M.open, { desc = "what each plugin's ticks have cost, most first" })

return M

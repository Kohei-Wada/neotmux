-- What each plugin's ticks cost: a list of every source, most expensive first, with how many
-- ticks it has run, their time together, the mean, the longest, and a sparkline of its last
-- ticks. The palette's `costs`.
--
-- The numbers are `neotmux.costs()`, wall time around each source's whole tick since the
-- config loaded — its drawing, its reads of panes, everything it did that frame. They are the
-- thing to look at before deciding a plugin draws too often, rather than a guess.
local list = require("neotmux.list")
local style = require("neotmux.style")
local g = neotmux.group("plugin.costs")

local M = {}

style.default("costs", { source = "mauve", number = "text", label = "overlay0", spark = "blue" })

local BARS = { "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█" }

-- The columns after the source's name: a heading, the width it is right-aligned in, and how
-- each row fills it.
local COLUMNS = {
  {
    "ticks",
    8,
    function(it)
      return string.format("%d", it.ticks)
    end,
  },
  {
    "total ms",
    10,
    function(it)
      return string.format("%.2f", it.total_ms)
    end,
  },
  {
    "mean",
    8,
    function(it)
      return string.format("%.2f", it.ticks > 0 and it.total_ms / it.ticks or 0)
    end,
  },
  {
    "longest",
    9,
    function(it)
      return string.format("%.2f", it.longest_ms)
    end,
  },
}

-- The widest source name among the rows shown, so every column after it starts at one place.
local name_width = 0
-- The longest recent tick of any source: every sparkline is drawn against it, so a bar means
-- the same time in every row.
local spark_top = 0

local function pad(text, width)
  return text .. string.rep(" ", width - neotmux.width(text))
end

local function right(text, width)
  return string.rep(" ", width - neotmux.width(text)) .. text
end

-- How many columns the name and the numbers take, and so where the sparkline starts.
local function numbers_width()
  local used = name_width + 2
  for _, column in ipairs(COLUMNS) do
    used = used + column[2]
  end
  return used
end

-- The last `room` of `samples` as bars, the newest at the right.
local function spark(samples, room)
  local out = {}
  for i = math.max(1, #samples - room + 1), #samples do
    local share = spark_top > 0 and samples[i] / spark_top or 0
    out[#out + 1] = BARS[math.max(1, math.ceil(share * #BARS))]
  end
  return table.concat(out)
end

local function header()
  local c = style.get("costs")
  local text = pad("source", name_width)
  for _, column in ipairs(COLUMNS) do
    text = text .. right(column[1], column[2])
  end
  return {
    { text = text, fg = c.label },
    { text = string.format("  last ticks, top %.2f ms", spark_top), fg = c.label },
  }
end

local function row(it, _, width)
  local c = style.get("costs")
  local numbers = ""
  for _, column in ipairs(COLUMNS) do
    numbers = numbers .. right(column[3](it), column[2])
  end
  return {
    { text = pad(it.source, name_width), fg = c.source },
    { text = numbers, fg = c.number },
    { text = "  " },
    { text = spark(it.recent_ms, width - numbers_width()), fg = c.spark },
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
      local rows = neotmux.costs()
      name_width, spark_top = neotmux.width("source"), 0
      for _, it in ipairs(rows) do
        name_width = math.max(name_width, neotmux.width(it.source))
        for _, sample in ipairs(it.recent_ms) do
          spark_top = math.max(spark_top, sample)
        end
      end
      return rows
    end,
    header = header,
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

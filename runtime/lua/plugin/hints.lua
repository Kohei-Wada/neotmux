-- What to do with a target `neotmux.hints` picked out.
--
--   C-b f       label the screen, copy what you point at
--   hint-send   label the screen, type what you point at into the focused pane
--   hint-open   label the screen, write an opener command for what you point at
--
-- Each is a `neotmux.command`, so the palette lists all three; only `hint-copy` also gets a
-- key, because copy is the one people want most.
--
-- `hint-open` cannot launch anything: this branch adds no host surface a plugin could ask to
-- start a process with, so it goes as far as the ABI reaches instead. It writes `M.opener`
-- followed by the target's text into the focused pane with `neotmux.pane.send`, and presses
-- Enter for it — exactly what a person would have typed to open the thing themselves. It does
-- not open anything; it types the command that would.
local hints = require("neotmux.hints")

local M = {}

-- The command `hint-open` hands a target to. `xdg-open` is the one Linux binary this can
-- assume; a user on another host reassigns it.
M.opener = "xdg-open"

local function pick(done)
  hints.open({ done = done })
end

neotmux.command.add("hint-copy", function()
  pick(function(target)
    local ok, err = neotmux.clipboard.set(target.text)
    if ok then
      neotmux.notify("info", "sent to the terminal: " .. target.text)
    else
      neotmux.notify("warn", "could not reach the terminal: " .. err)
    end
  end)
end, { desc = "label the screen, copy what you point at" })

neotmux.command.add("hint-send", function()
  pick(function(target)
    neotmux.pane.send(neotmux.pane.current(), target.text)
  end)
end, { desc = "label the screen, type what you point at into the focused pane" })

neotmux.command.add("hint-open", function()
  pick(function(target)
    neotmux.pane.send(neotmux.pane.current(), M.opener .. " " .. target.text .. "\n")
  end)
end, { desc = "label the screen, write an opener command for what you point at" })

if package.loaded["plugin.prefix"] or require("neotmux.defaults").prefix then
  require("plugin.prefix").commands["f"] = "hint-copy"
end

return M

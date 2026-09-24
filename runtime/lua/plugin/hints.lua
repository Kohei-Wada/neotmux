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
-- start a process with, so it goes as far as the ABI reaches instead. It writes `M.opener` and
-- the target's text, quoted for a POSIX shell, into the focused pane with `neotmux.pane.send`
-- — without pressing Enter, so it is left on the line for a person to read before they run it,
-- the same look they would have had typing it themselves. It does not open anything; it types
-- the command that would, and it is the person who decides to run it. The quoting is a best
-- effort, not a guarantee: what is actually reading that pane may not be a POSIX shell at all
-- — a REPL, a remote shell down an ssh session, anything — and single quotes mean nothing to
-- whatever that is.
--
-- `hint-send` and `hint-open` both write into the pane that is focused, not into the pane the
-- target came from — the point of `hint-send` is "type this where I am typing", and with hints
-- spanning every visible pane, that can be a different pane than the one you pointed at.
local hints = require("neotmux.hints")

local M = {}

-- The command `hint-open` hands a target to. `xdg-open` is the one Linux binary this can
-- assume; a user on another host reassigns it.
M.opener = "xdg-open"

local function pick(done)
  hints.open({ done = done })
end

-- Single-quoted for a POSIX shell: nothing inside single quotes is special except the single
-- quote itself, which cannot appear inside them at all, so a run of `'` is closed, escaped as
-- an ordinary quoted character, and reopened.
local function shell_quote(text)
  return "'" .. text:gsub("'", "'\\''") .. "'"
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
    neotmux.pane.send(neotmux.pane.current(), M.opener .. " " .. shell_quote(target.text))
  end)
end, { desc = "label the screen, write an opener command for what you point at" })

if package.loaded["plugin.prefix"] or require("neotmux.defaults").prefix then
  require("plugin.prefix").commands["f"] = "hint-copy"
end

return M

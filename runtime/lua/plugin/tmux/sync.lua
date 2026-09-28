-- Types into every pane of the window at once — tmux's `synchronize-panes`, as a plugin.
--
--   :sync-panes      turn it on or off, from the palette
--   C-b S            the same, from the prefix
--
-- While it is on, every key you press is relayed to each pane in the window you are looking
-- at, the focused one included, and the mode `sync` is raised so whoever draws modes says so.
-- The prefix is untouched: `plugin.tmux.prefix` sees a key before this does, so `C-b` still opens
-- the palette, the picker and everything else — which is what tmux does too, and what makes
-- the thing switchable off once it is on.
--
-- Nothing here is a host feature. `neotmux.pane.key` is the one piece that could not be Lua:
-- a keystroke's bytes depend on what the pane asked for — a pane in application cursor mode is
-- written `ESC O A` where another is written `ESC [ A` — so the spelling is the host's and the
-- deciding which panes get it is this file's.
--
-- Shipped on, which loads the plugin rather than the synchronising: until `:sync-panes` is
-- run, this file is a key handler that answers no. What is on by default is the command being
-- there to run.
local modes = require("neotmux.mode")
local g = neotmux.group("plugin.tmux.sync")

local M = {}

-- The name raised while it is on, for `plugin.footer` and anything else that reads modes.
M.mode = "sync"

local on = nil

local function panes()
  local window = neotmux.window.current()
  return window and neotmux.window.panes(window) or {}
end

function M.on()
  return on ~= nil
end

function M.toggle()
  if on then
    on:lower()
    on = nil
    return "sync off"
  end
  on = modes.raise(M.mode)
  return "sync on: every key goes to all " .. #panes() .. " panes of this window"
end

g.command.add("sync-panes", function()
  return M.toggle()
end, { desc = "type into every pane of this window at once" })

if
  package.loaded["plugin.tmux.prefix"]
  or require("neotmux.defaults").tmux and require("neotmux.defaults").prefix
then
  require("plugin.tmux.prefix").commands["S"] = "sync-panes"
end

g.on_key(function(key)
  if not on then
    return false
  end
  for _, pane in ipairs(panes()) do
    neotmux.pane.key(pane, key)
  end
  return true
end)

return M

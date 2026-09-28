-- Binding a key under the prefix, from a plugin that does not know whether the prefix is loaded.
--
-- A convention, not a plugin: it registers nothing.
--
--   local bind = require('neotmux.bind')
--   bind.prefix('t', 'timestamps')                  -- a command's name
--   bind.prefix('b', M.toggle, 'fold the sidebar')  -- or a function, and what it does
--
-- "Not loaded yet" is not "off". A plugin set up from your own `init.lua` — the way
-- `require('plugin.stamps').stall = ...` is — runs before the shipped layer has loaded the
-- prefix, and a plain `package.loaded` probe would leave its key unbound without a word. So the
-- prefix is loaded here when the defaults say it is about to be, which is what the shipped
-- layer was going to do anyway, and left alone only when you turned it off.
local M = {}

-- The prefix's table, or nil when it is turned off.
function M.prefix_table()
  local defaults = require("neotmux.defaults")
  if package.loaded["plugin.tmux.prefix"] or defaults.tmux and defaults.prefix then
    return require("plugin.tmux.prefix")
  end
end

-- `C-b key` runs `action`, a command's name or a function; `desc` is what `C-b ?` says it does,
-- and a command's name needs none, since the command says so itself.
function M.prefix(key, action, desc)
  local prefix = M.prefix_table()
  if prefix == nil then
    return
  end
  prefix.commands[key] = action
  if desc then
    prefix.descriptions[key] = desc
  end
end

return M

-- What tmux also does, loaded as one: the prefix key and its verbs, the status bar, copy mode
-- and its yank, the session picker, the borders between panes, and synchronized panes. Each is
-- still its own plugin under `plugin.tmux`, and its own field in `neotmux.defaults` turns it
-- off; `defaults.tmux = false` turns off all six.
--
-- The status bar first, as it reserves the row every other band is laid out after; the prefix
-- before the four that bind keys to it.
local defaults = require("neotmux.defaults")

for _, name in ipairs({ "status", "prefix", "copy", "sessions", "borders", "sync" }) do
  if defaults[name] then
    require("plugin.tmux." .. name)
  end
end

return {}

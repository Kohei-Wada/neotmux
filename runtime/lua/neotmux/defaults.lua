-- Which shipped plugins load. Every layer's `init.lua` runs, earliest layer first, and the
-- shipped layer is last — so setting a field here from your own `init.lua` is seen in time.
--
--   -- ~/.config/neotmux/init.lua
--   require('neotmux.defaults').status = false
--
-- This is a Lua convention, not a host feature: the host has no idea these defaults exist.
return {
  status = true,
  prefix = true,
  borders = true,
  -- With the first three rather than the last two: copy mode spends no screen space and no
  -- pane's cells, and a multiplexer that cannot read back over its own scrollback is missing
  -- the use of a buffer it is already keeping.
  copy = true,
  -- With copy mode, and for the same bargain: it spends no screen space until you press the
  -- key, and with more than one session going, a list of them with what each is running is
  -- the thing you would otherwise be keeping in your head.
  sessions = true,
  -- Off, unlike the rest: which output means a pane is waiting for a human is an opinion
  -- about what you run, and the other three are not.
  watch = false,
  -- Off for the same kind of reason, about space rather than output: a status line spends a
  -- row, a sidebar spends a third of the width.
  sidebar = false,
  -- Off for the same reason as the two above it: a row spent showing a mode is a row spent
  -- whether or not a mode is ever raised.
  footer = false,
}

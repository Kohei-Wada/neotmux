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
  -- Off, unlike the rest: which output means a pane is waiting for a human is an opinion
  -- about what you run, and the other three are not.
  watch = false,
}

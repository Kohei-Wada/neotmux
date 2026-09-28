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
  -- With them, for the same bargain again: nothing is on screen until `C-b :` is pressed, and
  -- every command any plugin has registered is something you would otherwise have to remember
  -- the key for, or not be able to reach at all.
  palette = true,
  -- With them: a notification nobody draws is a notification that only reaches the log, and
  -- the host stopped drawing them when what one looks like became a plugin's decision.
  notify = true,
  -- On, and note what that means: the plugin is loaded, not the synchronising. Loading it
  -- costs a key handler that answers no and a command in the palette; nothing is relayed
  -- anywhere until `:sync-panes` says so, and it says so again to stop.
  sync = true,
  -- With them, for the same bargain again: nothing is on screen until the key is pressed, and
  -- typing back a url or a hash you can already see is the thing you would otherwise be doing
  -- by hand.
  hints = true,
  -- On, and the one shipped plugin that makes a noise: the sounds ship with it, the state it
  -- reads is the detector's rather than an opinion about what you run, and a pane that starts
  -- waiting while you are looking somewhere else is the thing you would otherwise find out
  -- about ten minutes later.
  chime = true,
  -- On, and nothing but a command in the palette: what each plugin's ticks cost, which the host
  -- counts whether or not anyone asks.
  costs = true,
  -- On, and nothing on screen until `C-b t`: when every line of every pane arrived is kept
  -- from the start, since a time nobody wrote down is one nobody can show later.
  stamps = true,
  -- Off, unlike the four above: which command is worth stopping to ask about is an opinion
  -- about what you run, and one with no rules in it would stop nothing anyway.
  guard = false,
  -- Off, for the same kind of reason: which output means a pane is waiting for a human is an
  -- opinion about what you run, and the other three are not.
  watch = false,
  -- Off for the same kind of reason, about space rather than output: a status line spends a
  -- row, a sidebar spends a third of the width.
  sidebar = false,
  -- Off for the same reason as the two above it: a row spent showing a mode is a row spent
  -- whether or not a mode is ever raised.
  footer = false,
}

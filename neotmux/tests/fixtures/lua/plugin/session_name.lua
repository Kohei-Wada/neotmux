-- The name of the session this tick is for, in the log: the E2E proof that the fact reaches
-- the plugins at all. Asked from a tick rather than at load, because a load happens before
-- any session does and there is no name to be told there.
local said = false
neotmux.on_tick(function()
  if not said then
    said = true
    neotmux.log.info('session_name ' .. neotmux.session_name())
  end
end)

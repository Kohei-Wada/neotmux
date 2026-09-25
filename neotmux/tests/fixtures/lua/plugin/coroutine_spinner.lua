-- The same loop behind coroutine.wrap. A per-thread hook would silently not fire on a
-- thread mlua never registered, so this is the global hook, end to end (§5.1).
neotmux.on_tick(function()
  coroutine.wrap(function()
    while true do
    end
    neotmux.log.error("coroutine spinner ran past its loop")
  end)()
end)

return {}

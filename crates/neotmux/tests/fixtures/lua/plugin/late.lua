local late = {}

-- Registered from inside another plugin's dispatch: the owner is still this chunk, because
-- attribution is lexical. Raising is how the host is made to say so out loud — a tick error
-- is logged under the registration's owner.
function late.arm()
  local handle
  handle = neotmux.on_tick(function()
    handle:remove()
    error("late-callback-boom")
  end)
end

return late

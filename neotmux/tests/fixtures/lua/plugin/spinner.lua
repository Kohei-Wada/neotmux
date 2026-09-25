-- A tick that never returns. The log line after the loop is unreachable unless the budget
-- failed to cut the tick off, which is what makes that failure legible (§5.5).
neotmux.on_tick(function()
  while true do
  end
  neotmux.log.error("spinner ran past its loop")
end)

return {}

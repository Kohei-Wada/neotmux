-- One callback, many splits. Every one of them is host work — a fork, an exec, a terminal —
-- done after the tick the budget measured, so without a cap a plugin can buy as much
-- unmeasured host work as it likes by queueing it.
neotmux.on_key(function(key)
  if key == "C-f" then
    for _ = 1, 200 do
      neotmux.pane.split("vertical")
    end
    return true
  end
  return false
end)

return {}

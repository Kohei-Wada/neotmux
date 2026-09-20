-- Asks for a whole edge that is already partly spoken for. The refusal is a value rather
-- than an error: the call was legal and lost the rows to whoever asked first.
local handle, message = neotmux.reserve("bottom", 4)
if not handle then
  neotmux.log.error("greedy refused: " .. message)
end

neotmux.on_tick(function(ctx)
  if ctx:reserved() then
    neotmux.log.error("greedy drew in a band it was refused")
  end
end)

return {}

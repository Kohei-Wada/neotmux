local handle = neotmux.on_tick(function()
  neotmux.log.error("remover tick ran after its registration was removed")
end)

local first = pcall(function()
  handle:remove()
end)
local second = pcall(function()
  handle:remove()
end)

neotmux.log.info(
  "remover removed twice first=" .. tostring(first) .. " second=" .. tostring(second)
)

return {}

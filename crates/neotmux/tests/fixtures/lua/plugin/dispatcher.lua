local late = require("plugin.late")
local armed = false

neotmux.on_tick(function()
  if armed then
    return
  end
  armed = true
  late.arm()
end)

return {}

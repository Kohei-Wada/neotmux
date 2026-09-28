local defaults = require("neotmux.defaults")
for _, name in ipairs({ "chime", "watch", "sidebar" }) do
  if defaults[name] then
    require("plugin.herdr." .. name)
  end
end
return {}

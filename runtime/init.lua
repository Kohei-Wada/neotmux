-- What neotmux loads when it finds no reason not to. The search guard is what makes deleting
-- `lua/plugin/` a way of turning the defaults off rather than a missing-file error; the
-- defaults table is how a user turns one off without deleting anything.
for _, name in ipairs({ "status", "prefix" }) do
  local module = "plugin." .. name
  if package.searchpath(module, package.path) and require("neotmux.defaults")[name] then
    require(module)
  end
end

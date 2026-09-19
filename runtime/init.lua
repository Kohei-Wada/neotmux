-- What neotmux loads when it finds no reason not to. The search guard is what makes deleting
-- `lua/plugin/` a way of turning the defaults off rather than a missing-file error; the
-- defaults table is how a user turns one off without deleting anything.
if package.searchpath("plugin.status", package.path) and require("neotmux.defaults").status then
  require("plugin.status")
end

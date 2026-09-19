-- What neotmux loads when it finds no reason not to. The search guard is what makes
-- deleting `lua/plugin/` a way of turning the defaults off rather than a missing-file error.
if package.searchpath("plugin.status", package.path) then
  require("plugin.status")
end

-- What neotmux loads when it finds no reason not to. The search guard is what makes deleting
-- `lua/plugin/` a way of turning the defaults off rather than a missing-file error; the
-- defaults table is how a user turns one off without deleting anything.
for _, name in ipairs({
  "status",
  "prefix",
  "copy",
  "sessions",
  "palette",
  "notify",
  "borders",
  "guard",
  "sync",
  "watch",
  "sidebar",
  "footer",
  "hints",
  "chime",
  "costs",
  "stamps",
}) do
  local module = "plugin." .. name
  if package.searchpath(module, package.path) and require("neotmux.defaults")[name] then
    require(module)
  end
end

-- Here rather than in a plugin: turning every shipped plugin off must not take the way back.
neotmux.group("neotmux.reload").command.add("config-reload", function()
  neotmux.reload()
end, { desc = "run the config again, keeping the panes" })

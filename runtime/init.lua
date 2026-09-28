-- What neotmux loads when it finds no reason not to. The search guard is what makes deleting
-- `lua/plugin/` a way of turning the defaults off rather than a missing-file error; the
-- defaults table is how a user turns one off without deleting anything.
--
-- What tmux also does — the prefix, the status bar, copy mode, the session picker, the borders,
-- synchronized panes — is `plugin.tmux`, which loads its own. What follows agents the way herdr
-- does — the sidebar, the chime, the watch rules — is `plugin.herdr`. The rest is neotmux's own.
for _, name in ipairs({
  "tmux",
  "herdr",
  "palette",
  "notify",
  "guard",
  "footer",
  "hints",
  "costs",
  "stamps",
  "exits",
  "theme",
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

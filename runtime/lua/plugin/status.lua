-- The default status line. It lives here, and not in the host, because the host does not
-- know what a status line says — only how to draw the row one asks for.
neotmux.on_tick(function(ctx)
  local panes = neotmux.pane.list()
  ctx:status("panes " .. #panes .. " focus " .. tostring(neotmux.pane.current()))
end)

return {}

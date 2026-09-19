local reported = false

neotmux.on_tick(function()
  if reported then
    return
  end
  reported = true

  local ids = neotmux.pane.list()
  neotmux.log.info(
    "panes list=" .. #ids .. " first=" .. tostring(ids[1]) .. " current=" .. tostring(neotmux.pane.current())
  )

  local ok, err = pcall(neotmux.pane.split, "sideways")
  neotmux.log.info("panes bad direction ok=" .. tostring(ok) .. " err=" .. tostring(err))

  neotmux.pane.split("vertical")
  neotmux.pane.focus(ids[1])
  -- Ops apply after the tick (§4.6), so the split must not be visible to the caller yet.
  neotmux.log.info("panes list after split=" .. #neotmux.pane.list())
end)

return {}

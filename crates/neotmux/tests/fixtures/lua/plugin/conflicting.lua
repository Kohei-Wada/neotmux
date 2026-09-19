local handle, message = neotmux.keymap.set("C-x", function()
  return true
end)

if handle == nil then
  neotmux.log.info("conflicting refused: " .. message)
else
  neotmux.log.error("conflicting bound C-x, which binder already holds")
end

return {}

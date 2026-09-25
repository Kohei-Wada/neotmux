local late = {}

-- Written here but registered by plugin/dispatcher, from inside its own tick. Attribution
-- is lexical, so the owner is this chunk; raising is how the host says so out loud, since a
-- tick error is logged under the registration's owner.
function late.boom()
  late.handle:remove()
  error("late-callback-boom")
end

return late

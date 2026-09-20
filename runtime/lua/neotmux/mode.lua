-- A place for a mode to be named, in the same spirit as `neotmux.defaults`: the host has
-- never heard of this table, and nothing but convention makes a plugin use it.
--
-- A plugin that takes the keyboard modally — the reason `on_key` returns `true` at all — calls
-- `raise` on the way in and `lower` on the way out. A reader calls `current` and gets whichever
-- name is on top, with no way to find out who put it there. That is what lets a third-party
-- mode plugin show up in `plugin.footer` for free: the footer only ever calls `current`.
--
-- Raising returns a handle, the same shape `neotmux.reserve` and every `on_*` registration
-- already return. That is what answers the question a shared name table would raise on its
-- own: two plugins can raise the very same name, and each can only lower the entry it was
-- handed back, never the other one's. Lowering is idempotent, like `handle:remove()` elsewhere
-- in the ABI — a second `lower()` on a handle already lowered finds nothing to remove.
--
-- Raised modes stack. `current()` answers the most recently raised name still up, so if B
-- raises while A's is showing, the reader sees B; when B lowers, A's is still there and shows
-- again. A lowers its own by calling its own handle, which never touches B's entry regardless
-- of what either named itself.
--
-- Nothing here catches a plugin that forgets to lower. That is by design, not an oversight:
-- there is no host-side hook to reclaim a mode on unload or on error, because there is no
-- host-side concept of a mode to reclaim it with — the same bargain `neotmux.reserve` makes
-- with a band nobody gives back. A plugin that raises and never lowers leaves its name on top
-- forever, and every reader, footer included, keeps showing it. That is a bug in the plugin,
-- the same way leaking a reserved band is, and it is caught the same way: by looking at the
-- screen.
local M = {}

local stack = {}

-- The most recently raised name still up, or nil when the stack is empty.
function M.current()
  local top = stack[#stack]
  return top and top.name
end

-- Puts `name` on top and returns a handle whose `lower()` takes back exactly this entry,
-- wherever it ends up in the stack by the time that happens.
function M.raise(name)
  local entry = { name = name }
  stack[#stack + 1] = entry

  local handle = {}
  function handle:lower()
    for i = #stack, 1, -1 do
      if stack[i] == entry then
        table.remove(stack, i)
        return
      end
    end
  end
  return handle
end

return M

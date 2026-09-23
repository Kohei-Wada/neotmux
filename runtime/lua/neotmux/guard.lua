-- Blocking a write by what it says, on top of `neotmux.on_write`.
--
-- The host decides that a handler which refuses, raises or overruns stops the write. *What* is
-- worth stopping is an opinion, and an opinion belongs here — beside `neotmux.mode` and
-- `neotmux.box` — rather than in the host. Replace this file with your own and nothing about
-- neotmux changes.
--
--   local guard = require('neotmux.guard')
--   guard.only('tool')                                 -- agents, not the person's plugins
--   guard.block('rm%s+%-%w*[rf]', 'rm -rf is blocked')
--
-- One handler is registered, the first time a rule is added: a table of patterns read on every
-- write, so adding a rule later needs no new subscription.
local M = {}

-- Lua patterns, each with what to say when it matches. Read on every write, so a rule added
-- from your own init.lua after load takes effect.
M.patterns = {}

-- Which writers the rules apply to, as `neotmux.on_write`'s `from`. `nil` is all of them.
M.from = nil

local registered = false

local function answer(write)
  if M.from and write.from ~= M.from then
    return
  end
  for _, rule in ipairs(M.patterns) do
    if write.text:find(rule.pattern) then
      return false, rule.why
    end
  end
end

local function arm()
  if registered then
    return
  end
  registered = true
  neotmux.on_write(answer)
end

-- A pattern to refuse, and the sentence whoever wrote it is told.
function M.block(pattern, why)
  M.patterns[#M.patterns + 1] = { pattern = pattern, why = why or ("'" .. pattern .. "' is blocked") }
  arm()
end

-- Narrows every rule to one kind of writer: 'tool' for a connection that is not a client,
-- 'plugin' for a plugin's own write.
function M.only(from)
  M.from = from
  arm()
end

return M

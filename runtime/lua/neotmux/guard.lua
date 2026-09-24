-- What is worth stopping, and the two places it is asked.
--
-- The host decides that an `on_write` handler which refuses, raises or overruns stops the
-- write. *What* is worth stopping is an opinion, and an opinion belongs here — beside
-- `neotmux.mode` and `neotmux.box` — rather than in the host. Replace this file with your own
-- and nothing about neotmux changes.
--
--   local guard = require('neotmux.guard')
--   guard.block('rm%s+%-%w*[rf]', 'rm -rf is blocked')
--   guard.block(guard.plain('git push --force'), 'no force pushes', 'tool')  -- agents only
--
-- One rule table, asked from two sides. `on_write` is the agent's: a write whose text a rule
-- matches never reaches the pty, and this file registers that handler itself the first time a
-- rule is added. `plugin.guard` is the person's: it reads the line being typed and swallows
-- the Enter that would run it, which is a different mechanism for the same list, and it asks
-- through `verdict`.
--
-- A rule is a pattern or a function. A pattern is matched against the text; a function is
-- handed `{ pane =, text =, from = }` and answers a sentence or nothing, which is how a rule
-- like "anything at all typed into a pane running `ssh prod-…`" is written — the host has no
-- notion of production and this file should not invent one.
--
-- `block`'s first argument is a Lua pattern, not the literal string it looks like, and the two
-- disagree about what a character like `-` means. `guard.block('rm -rf', 'blocked')` never
-- matches `rm -rf /tmp/x` — `-` is "zero or more of the previous item, preferring fewer", so
-- the pattern reads as "rm, then any number of spaces, then rf" and the space before the `-`
-- in the real command breaks that — and it does match `rmrf`, which was never the command
-- anyone meant to name. A rule that looks armed and is not is the worst failure a checkpoint
-- can have, so a literal command is written with `guard.plain`, not typed as a string on its
-- own.
local M = {}

-- Lua patterns, each with what to say when it matches and which writer it is for. Read on every
-- ask, so a rule added from your own init.lua after load takes effect.
M.patterns = {}

-- The `from` given to rules added from here on, or `nil` for "every writer". It is a default
-- for what comes next rather than a filter over the table, because the table has two readers:
-- an `only('tool')` that also silenced the rules a *person's* Enter is checked against would be
-- one file turning off another's guard, which is the failure this whole thing is for.
M.from = nil

-- Rules that are functions rather than patterns, in the order they were added.
M.rules = {}

local registered = false

-- Why this text is worth stopping, or nil. `what` is `{ pane =, text =, from = }`; the two
-- callers fill in what they know, and a rule that needs a field a caller did not have is a
-- rule that does not fire for that caller.
function M.verdict(what)
  for _, rule in ipairs(M.patterns) do
    if (not rule.from or rule.from == what.from) and what.text:find(rule.pattern) then
      return rule.why
    end
  end
  for _, rule in ipairs(M.rules) do
    if not rule.from or rule.from == what.from then
      local why = rule.fn(what)
      if why then
        return why
      end
    end
  end
  return nil
end

local function answer(write)
  local why = M.verdict(write)
  if why then
    return false, why
  end
end

local function arm()
  if registered then
    return
  end
  registered = true
  neotmux.on_write(answer)
end

-- A pattern to refuse, and the sentence whoever wrote it is told. `from` narrows this one rule
-- to one writer; left out, it takes whatever `only` was last set to.
function M.block(pattern, why, from)
  M.patterns[#M.patterns + 1] = {
    pattern = pattern,
    why = why or ("'" .. pattern .. "' is blocked"),
    from = from or M.from,
  }
  arm()
end

-- A rule the patterns cannot express: a function over `{ pane =, text =, from = }` answering
-- the sentence to say, or nothing. Where a pattern reads the text alone, this can read the
-- pane — `neotmux.pane.info(what.pane).command` is how "this pane is on production" is asked.
function M.rule(fn, from)
  M.rules[#M.rules + 1] = { fn = fn, from = from or M.from }
  arm()
end

-- Which writer the rules added *after* this are for: 'tool' for a connection that is not a
-- client, 'plugin' for a plugin's own write, 'terminal' for a person's own Enter, `nil` for all
-- of them. It sets a default rather than filtering the table, so two files can hold rules for
-- two audiences without either silencing the other.
function M.only(from)
  M.from = from
  arm()
end

-- A literal turned into a pattern that means exactly itself: every character a Lua pattern
-- would otherwise give a meaning to is escaped out of it, so
-- `guard.block(guard.plain('git push --force'), 'no force pushes')` blocks the text it names
-- and nothing that only looks like it.
function M.plain(text)
  return (text:gsub("%W", "%%%0"))
end

return M

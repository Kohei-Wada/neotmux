-- Swallows the Enter that would run something dangerous, once.
--
-- The rules are `neotmux.guard`'s, shared with the agent-facing half: whatever a write from
-- `neotmux send` is refused for, a line you typed is stopped for too. This file is only the
-- other mechanism — a person's Enter is a key, not a write, and a key is all the host will
-- ever say about what is being typed.
--
--   Enter on a line a rule matches   swallowed, and the mode `guard` goes up
--   Enter again within `M.grace`     goes through as typed
--
-- Off by default, like `watch`: which command is worth a second thought is an opinion about
-- what you run.
--
-- What makes this honest rather than a guess is `neotmux.pane.mode`. Two states it refuses to
-- have an opinion in:
--
--   canonical and not echo   a password prompt. Reading the screen there is the one thing
--                            this must never do, and stopping a key is the one thing it must
--                            never do either.
--   alternate                a full-screen program. Enter is not a command in vim.
--
-- And `neotmux.pane.cursor` is which row is being typed, which before it existed could only
-- be guessed as the last non-blank line.
--
-- The limit that remains, and it is a real one: what a rule sees is the row as drawn on the
-- screen, prompt and all. A prompt with `rm -rf` in it matches; a command wrapped over two
-- rows is seen in part; text a program never echoed is not seen at all. A guard on the keys
-- is a second pair of eyes, not a proof.
local guard = require("neotmux.guard")
local modes = require("neotmux.mode")
local g = neotmux.group("plugin.guard")

local M = {}

-- How long the second Enter has to arrive for it to count as "yes, I meant it".
M.grace = 5

-- What the mode is called while one is held back, for whoever draws modes (`plugin.footer`).
M.mode = "guard"

local armed = nil

local function disarm()
  if armed then
    armed.mode:lower()
    armed = nil
  end
end

-- The row the cursor is on, which is the line being typed wherever a line is being typed at
-- all. `nil` for a pane that said nothing this frame — one that has just closed.
local function line_of(pane)
  local at = neotmux.pane.cursor(pane)
  if not at then
    return nil
  end
  local lines = neotmux.pane.content(pane)
  return lines and lines[at.y + 1]
end

-- Whether this pane is one to have an opinion about at all.
local function readable(mode)
  if not mode or mode.alternate then
    return false
  end
  return not (mode.canonical and mode.echo == false)
end

g.on_key(function(key)
  if key ~= "Enter" then
    return false
  end
  local pane = neotmux.pane.current()
  if not pane or not readable(neotmux.pane.mode(pane)) then
    disarm()
    return false
  end
  local line = line_of(pane)
  if not line then
    disarm()
    return false
  end
  -- The same line, held back a moment ago: this Enter is the one that means it.
  if armed and armed.pane == pane and armed.line == line and os.time() - armed.at <= M.grace then
    disarm()
    return false
  end
  local why = guard.verdict({ pane = pane, text = line, from = "terminal" })
  if not why then
    disarm()
    return false
  end
  disarm()
  armed = { pane = pane, line = line, at = os.time(), mode = modes.raise(M.mode) }
  neotmux.notify("warn", why .. " — Enter again within " .. M.grace .. "s to run it")
  return true
end)

return M

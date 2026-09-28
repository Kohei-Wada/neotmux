-- A pane's commands, out of the marks its shell sent (OSC 133): where each one's prompt,
-- command line and output start, and how it ended.
--
-- A convention, not a plugin: it registers nothing, and reads `neotmux.pane.marks` when asked.
--
--   local commands = require('neotmux.commands')
--   for _, c in ipairs(commands.of(pane)) do
--     -- c.prompt, c.input, c.output, c.done: line numbers, the way `neotmux.pane.history`
--     -- counts them, any but `prompt` possibly nil; c.exit: the status `done` carried
--     -- c.state: 'done', 'running', 'prompt' or 'unknown'
--   end
--
-- The marks are the shell's word and nothing else, so they are read as hints. A shell that
-- sends none has no commands. A command whose `done` never came — `ssh` or `bash --norc` run
-- inside a shell that marks, where the next prompt is the outer shell's, or a `done` with no
-- `output` before it, which an empty Enter or Ctrl-C at a prompt sends (bash's `PS0` is not
-- shown for an empty line) — is `unknown` once a prompt follows it, never `running` forever.
-- Only the newest command can be `running`, and it is for as long as it runs: there is no
-- timeout.
local M = {}

function M.of(pane)
  local out, current = {}, nil
  for _, mark in ipairs(neotmux.pane.marks(pane) or {}) do
    if mark.kind == "prompt" then
      if current and current.state == nil then
        current.state = "unknown"
      end
      current = { prompt = mark.line }
      out[#out + 1] = current
    elseif current and current.state == nil then
      if mark.kind == "input" then
        current.input = current.input or mark.line
      elseif mark.kind == "output" then
        current.output = current.output or mark.line
      elseif mark.kind == "done" and current.output then
        current.done = mark.line
        current.exit = mark.exit
        current.state = "done"
      end
    end
  end
  if current and current.state == nil then
    current.state = current.output and "running" or "prompt"
  end
  return out
end

-- The newest command, or nil for a pane with none.
function M.last(pane)
  local all = M.of(pane)
  return all[#all]
end

return M

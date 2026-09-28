-- One line of text, asked for in a float.
--
-- The host has no such thing and does not need one: a float, a mode, an `on_key` over a local
-- and a caret drawn as a rect are the whole of it. It lives here rather than in the plugin that
-- wanted it first so that the next plugin to want one does not write it again.
--
--   require('neotmux.prompt').open({
--     title = 'rename window',
--     value = neotmux.window.name(id) or '',
--     done = function(text) neotmux.window.rename(id, text ~= '' and text or nil) end,
--   })
--
-- `done` is called with what was typed when Enter is pressed, and not at all when Esc is. The
-- line is edited with `neotmux.line`'s keys — `Backspace`, `C-h`, `C-w`, `C-u` — as every other
-- is. The
-- terminal's own cursor belongs to the focused pane, so the caret is drawn — which is what a
-- text field is anywhere else too.
local modes = require("neotmux.mode")
local box = require("neotmux.box")
local line = require("neotmux.line")
local style = require("neotmux.style")
local g = neotmux.group("neotmux.prompt")

local M = {}

style.default("float.prompt", {
  frame = "mantle",
  text = "text",
  caret = "yellow",
  hint = "surface2",
})

M.width = 40

local state = nil
local win = nil
local mode_handle = nil

local function close()
  if win then
    win:remove()
    win = nil
  end
  if mode_handle then
    mode_handle:lower()
    mode_handle = nil
  end
  state = nil
end

-- Where the panes are, as of the last tick: the prompt is centred over them rather than over
-- the terminal, so it does not sit on the status bar or a sidebar.
local area = nil

local function wanted()
  local w = math.min(M.width, area and area.w or M.width)
  local x = (area and area.x or 0) + math.max(0, math.floor(((area and area.w or w) - w) / 2))
  local y = (area and area.y or 0) + math.max(0, math.floor(((area and area.h or 3) - 3) / 2))
  return { x = x, y = y, w = w, h = 3 }
end

-- Opens the prompt, replacing one already up. `opts.title` names it, `opts.value` is what the
-- field starts with, `opts.done` is called with the text.
function M.open(opts)
  close()
  state = { title = opts.title or "", value = opts.value or "", done = opts.done }
  local handle = g.float(wanted())
  if not handle then
    state = nil
    return
  end
  win = handle
  mode_handle = modes.raise(opts.mode or "prompt")
end

function M.is_open()
  return state ~= nil
end

g.on_key(function(key)
  if state == nil then
    return false
  end
  if key == "Esc" then
    close()
  elseif key == "Enter" then
    local done, value = state.done, state.value
    close()
    if done then
      done(value)
    end
  else
    state.value = line.edit(state.value, key) or state.value
  end
  return true
end)

g.on_tick(function(ctx)
  local left, top, right, bottom
  for _, pane in ipairs(neotmux.pane.list()) do
    local r = ctx:pane_outer(pane)
    if r then
      left = math.min(left or r.x, r.x)
      top = math.min(top or r.y, r.y)
      right = math.max(right or (r.x + r.w), r.x + r.w)
      bottom = math.max(bottom or (r.y + r.h), r.y + r.h)
    end
  end
  if left then
    area = { x = left, y = top, w = right - left, h = bottom - top }
  end

  if state == nil or win == nil then
    return
  end
  local rect = ctx:float_rect(win)
  if rect == nil then
    return
  end
  local want = wanted()
  if rect.w ~= want.w or rect.h ~= want.h then
    win:resize_to(want.w, want.h)
  end
  if rect.x ~= want.x or rect.y ~= want.y then
    win:move_to(want.x, want.y)
  end

  local c = style.get("float.prompt")
  local inner = box.draw(ctx, win, rect, { title = state.title, bg = c.frame })
  ctx:text(win, inner.x + 1, inner.y, state.value, {
    fg = c.text,
    bg = c.frame,
  })
  ctx:rect(win, inner.x + 1 + neotmux.width(state.value), inner.y, 1, 1, { bg = c.caret })
end)

return M

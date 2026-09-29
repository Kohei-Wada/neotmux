-- The session picker: every session there is, which one you are in, what each is running —
-- and a way to make one, go to one, or end one.
--
--   C-b s       open it
--   j k         the cursor, one row
--   Enter       go to the session under the cursor
--   c           make one: type a name, Enter to make it and go there, Esc to think again
--   r           rename the one under the cursor, the same way
--   d           end the one under the cursor
--   Esc q       leave
--
--   C-b N       make one without the picker, called the next number, and go there
--   C-b W       rename the one you are in
--   C-b X       end the one you are in, going to another first if there is one
--
-- A composition on `neotmux.list`: the float, the cursor, the scrolling and the caret are
-- that module's. What is left here is what makes this list a session picker rather than any
-- other one — what a row says, and what `Enter`, `c`, `r` and `d` do. The frame, the cursor row
-- and the caret are coloured through `style.get('float.list')`, not this file's.
local list = require("neotmux.list")
local prompt = require("neotmux.prompt")
local pane_state = require("neotmux.state")
local style = require("neotmux.style")
local g = neotmux.group("plugin.tmux.sessions")

-- What a session made with no name is called, as the host calls one started that way: the
-- lowest number no running session is.
local function next_number()
  local taken = {}
  for _, name in ipairs(neotmux.session.list()) do
    taken[name] = true
  end
  local n = 0
  while taken[tostring(n)] do
    n = n + 1
  end
  return tostring(n)
end

local M = {}

style.default("sessions", {
  name = "text",
  current = "green", -- the session you are in
  working = "green",
  blocked = "yellow", -- the one that wants you
  idle = "surface2",
  doing = "overlay0", -- what it is running, quieter than its name
})

-- How wide the window is, and the most rows of it the list may take. Both are read on the way
-- in, so setting them from your own `init.lua` takes effect at the next open.
M.width = 40
M.rows = 10

-- What a session is doing: how many panes it has, and what the one somebody is looking at is
-- running. Read through `with`, which is what lets a picker say anything at all about a
-- session nobody is attached to.
local function doing(name)
  local panes, command, worst = 0, nil, "idle"
  neotmux.session.with(name, function()
    local here = neotmux.pane.list()
    panes = #here
    local focus = neotmux.pane.current()
    local info = focus and neotmux.pane.info(focus)
    command = info and info.command
    worst = pane_state.worst(here)
  end)
  return panes, command, worst
end

-- A name a session may have, one character at a time. `neotmux.list` asks this only for a
-- character that already passed its own printability test, so refusing a leading `-` is the
-- one thing this picker has ever had an opinion of its own about.
local function nameable(key, text)
  return not (key == "-" and text == "")
end

-- One row: the state icon, the name — green for the one you are in — and, where the row is
-- wide enough to hold both, what it is running on the right. A name too long for the row
-- keeps the row: the right column is dropped instead, because it is the one you can do
-- without.
local function row(name)
  local c = style.get("sessions")
  local panes, command, worst = doing(name)
  local said = (command or "-") .. "  " .. panes .. "p"
  local runs = {
    { text = pane_state.icon(worst), fg = c[worst] },
    { text = " " },
    {
      text = name,
      fg = name == neotmux.session.current() and c.current or c.name,
    },
  }
  -- The inner width a row has to work with is `neotmux.list`'s own placement to know, not
  -- this file's; `M.width` is the one figure this picker already owns, and the frame it asks
  -- for is never wider than the terminal, so this is exact whenever the terminal holds it and
  -- only ever short of it on a terminal too small for `M.width` in the first place.
  local room = M.width - 2 - 2 - neotmux.width(name) - neotmux.width(said) - 1
  if room > 1 then
    runs[#runs + 1] = { text = string.rep(" ", room) }
    runs[#runs + 1] = { text = said, fg = c.doing }
  end
  return runs
end

function M.open()
  -- Names ended since this open that the host has not caught up with yet: `d` reads like it
  -- worked at once, the same as it always has, rather than waiting a frame for the list it
  -- came from to change out from under it.
  local killed = {}

  list.open({
    title = "sessions",
    mode = "sessions",
    width = M.width,
    rows = M.rows,
    items = function()
      local names = {}
      for _, name in ipairs(neotmux.session.list()) do
        if not killed[name] then
          names[#names + 1] = name
        end
      end
      return names
    end,
    row = row,
    footer = function()
      return "j k  enter  c new  r name  d kill"
    end,
    done = function(name, done_handle)
      if name ~= neotmux.session.current() then
        neotmux.session.switch(name)
      end
      done_handle:close()
    end,
    keys = {
      ["c"] = function(_, ask_handle)
        ask_handle:ask({
          label = "name",
          accept = nameable,
          done = function(text)
            local name = text ~= "" and text or next_number()
            neotmux.session.create(name)
            neotmux.session.switch(name)
            ask_handle:close()
          end,
        })
      end,
      ["r"] = function(name, ask_handle)
        if not name then
          return
        end
        ask_handle:ask({
          label = "rename",
          value = name,
          accept = nameable,
          done = function(text)
            if text ~= "" and text ~= name then
              neotmux.session.rename(name, text)
            end
          end,
        })
      end,
      ["d"] = function(name)
        -- The one you are in is refused rather than ended: ending it would leave this client
        -- with nothing to be attached to, and the picker with nothing to close over.
        if name and name ~= neotmux.session.current() then
          neotmux.session.kill(name)
          killed[name] = true
        end
      end,
    },
  })
end

-- The picker's `c` without the picker, and without asking: the name is the next number, and
-- `session-rename` is there for one that should be called something else.
g.command.add("session-new", function()
  local name = next_number()
  neotmux.session.create(name)
  neotmux.session.switch(name)
end, { desc = "make a session and go there" })

-- The picker's `r` without the picker, for the palette: the new name asked for in a prompt.
g.command.add("session-rename", function()
  local name = neotmux.session.current()
  if not name then
    return
  end
  prompt.open({
    title = "rename session",
    value = name,
    mode = "rename",
    done = function(text)
      if text ~= "" and text ~= name then
        neotmux.session.rename(name, text)
      end
    end,
  })
end, { desc = "rename the session you are in" })

-- The client is moved before the session goes, so it lands somewhere rather than being let
-- go; with no other session there is nowhere to land, and the server ends with it.
g.command.add("session-kill", function()
  local name = neotmux.session.current()
  if not name then
    return
  end
  for _, other in ipairs(neotmux.session.list()) do
    if other ~= name then
      neotmux.session.switch(other)
      break
    end
  end
  neotmux.session.kill(name)
end, { desc = "end the session you are in, going to another first" })

local bind = require("neotmux.bind")
bind.prefix("s", function()
  M.open()
end, "pick a session to switch to, make or end one")
bind.prefix("N", "session-new")
bind.prefix("W", "session-rename")
bind.prefix("X", "session-kill")

return M

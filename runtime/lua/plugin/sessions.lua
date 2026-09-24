-- The session picker: every session there is, which one you are in, what each is running —
-- and a way to make one, go to one, or end one.
--
--   C-b s       open it, again to leave without choosing
--   j k         the cursor, one row
--   Enter       go to the session under the cursor
--   c           make one: type a name, Enter to make it and go there, Esc to think again
--   r           rename the one under the cursor, the same way
--   d           end the one under the cursor
--   Esc q       leave
--
-- A composition on `neotmux.list`: the float, the cursor, the scrolling and the caret are
-- that module's. What is left here is what makes this list a session picker rather than any
-- other one — what a row says, and what `Enter`, `c`, `r` and `d` do.
local list = require("neotmux.list")
local pane_state = require("neotmux.state")

local M = {}

-- Read every tick, so assigning to one from your own `init.lua` takes effect.
M.colors = {
  name = "#cdd6f4",
  current = "#a6e3a1", -- green: the session you are in
  working = "#a6e3a1", -- green
  blocked = "#f9e2af", -- yellow: the one that wants you
  idle = "#585b70", -- surface2
  doing = "#6c7086", -- overlay0: what it is running, quieter than its name
}

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
  local panes, command, worst = doing(name)
  local said = (command or "-") .. "  " .. panes .. "p"
  local runs = {
    { text = pane_state.icon(worst), fg = M.colors[worst] },
    { text = " " },
    {
      text = name,
      fg = name == neotmux.session.current() and M.colors.current or M.colors.name,
    },
  }
  -- The inner width a row has to work with is `neotmux.list`'s own placement to know, not
  -- this file's; `M.width` is the one figure this picker already owns, and the frame it asks
  -- for is never wider than the terminal, so this is exact whenever the terminal holds it and
  -- only ever short of it on a terminal too small for `M.width` in the first place.
  local room = M.width - 2 - 2 - neotmux.width(name) - neotmux.width(said) - 1
  if room > 1 then
    runs[#runs + 1] = { text = string.rep(" ", room) }
    runs[#runs + 1] = { text = said, fg = M.colors.doing }
  end
  return runs
end

-- The list up right now, or nil the rest of the time — kept so a second `open()` while one is
-- already up closes it, the same toggle `C-b s` has always been.
local handle = nil

function M.open()
  if handle and handle:selected() ~= nil then
    handle:close()
    handle = nil
    return
  end

  -- Names ended since this open that the host has not caught up with yet: `d` reads like it
  -- worked at once, the same as it always has, rather than waiting a frame for the list it
  -- came from to change out from under it.
  local killed = {}

  handle = list.open({
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
            if text == "" then
              return
            end
            neotmux.session.create(text)
            neotmux.session.switch(text)
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

if package.loaded["plugin.prefix"] then
  require("plugin.prefix").commands["s"] = function()
    M.open()
  end
end

return M

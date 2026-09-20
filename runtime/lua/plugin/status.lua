-- The default status bar, on one reserved row across the top.
--
-- It lives here, and not in the host, because the host does not know what a status bar is.
-- What it knows is how to take a band of rows away from the panes on request: `reserve`
-- returns a handle like every other registration, the panes get what is left, and nothing a
-- shell writes can land in the band while nothing in the band can be overwritten by a pane.
--
-- Drawing into it is the ordinary `ctx:text` with no pane, because `ctx:reserved()` answers
-- in the surface's coordinates — the same ones a `nil` pane draws in.
--
-- On the left, the windows. On the right, what the focused pane is running and where. The
-- bar is composed from scratch every tick, so it follows the focus, a command and a `cd`
-- with no keystroke to prompt it.
local segments = require("status.segments")
local path = require("status.path")

local M = {}

-- Catppuccin Mocha, the palette this bar was drawn from. Every field is read on the tick, so
-- assigning to one from your own init.lua takes effect.
M.colors = {
  bar = "#181825", -- mantle: the band everything else sits on
  current = "#cba6f7", -- mauve: the cap on the window being shown
  inactive = "#313244", -- surface0: the cap on every other window
  on_current = "#181825",
  on_inactive = "#cdd6f4",
  body = "#45475a", -- surface1: the current window's own block
  body_text = "#cdd6f4",
  module = "#313244",
  module_text = "#cdd6f4",
  icon = "#11111b", -- crust: an icon, on its accent
  command = "#eba0ac", -- maroon
  directory = "#a6e3a1", -- green
}

-- Nerd Font glyphs. Without the font, one line in your own init.lua takes them away:
--
--   require('plugin.status').caps = { left = '', right = '' }
--   require('plugin.status').icons = { command = '', directory = '' }
--
-- Written as escapes rather than as the characters themselves so the file stays readable in
-- an editor that has no such font.
M.caps = { left = "\u{e0b6}", right = "\u{e0b4}" }
M.icons = { command = "\u{f489}", directory = "\u{f4d4}" }

-- `$HOME`, for the `~` a shell prompt writes. The VM has no `os.getenv`, so assign to this
-- from your own init.lua if the search path does not carry it either.
M.home = path.home(package.path)

M.band = neotmux.reserve("top", 1)

local function append(runs, more)
  for _, run in ipairs(more) do
    runs[#runs + 1] = run
  end
end

-- tmux caps its active window and leaves the rest flat, and a window's name sits beside its
-- number. The only name neotmux has for a window is what its focused pane is running, and
-- `pane.info` answers about the current window alone — so the one being shown is the one
-- that gets a name.
local function window_runs(name)
  local runs = {}
  local current = neotmux.window.current()
  for _, id in ipairs(neotmux.window.list()) do
    local is_current = id == current
    local accent = is_current and M.colors.current or M.colors.inactive
    append(runs, {
      { text = M.caps.left, fg = accent, bg = M.colors.bar },
      {
        text = id .. " ",
        fg = is_current and M.colors.on_current or M.colors.on_inactive,
        bg = accent,
      },
    })
    if is_current and name then
      append(runs, {
        { text = " " .. name, fg = M.colors.body_text, bg = M.colors.body },
        { text = M.caps.right, fg = M.colors.bar, bg = M.colors.body },
      })
    else
      runs[#runs + 1] = { text = M.caps.right, fg = M.colors.bar, bg = accent }
    end
    runs[#runs + 1] = { text = " ", fg = M.colors.bar, bg = M.colors.bar }
  end
  return runs
end

local function module_runs(icon, accent, text)
  return {
    { text = M.caps.left, fg = accent, bg = M.colors.bar },
    { text = icon .. " ", fg = M.colors.icon, bg = accent },
    { text = " " .. text .. " ", fg = M.colors.module_text, bg = M.colors.module },
  }
end

-- What a module costs beside the text it is there to show: its cap, its icon and the three
-- spaces around the two.
local function module_chrome(icon)
  return segments.columns(M.caps.left) + segments.columns(icon) + 3
end

neotmux.on_tick(function(ctx)
  local row = ctx:reserved()
  -- Nil while the terminal is too small to spare the row. The panes have it back, so there
  -- is nowhere to draw and nothing to draw over.
  if not row then
    return
  end
  ctx:rect(nil, row.x, row.y, row.w, row.h, { bg = M.colors.bar })

  local focused = neotmux.pane.current()
  -- A pane whose facts could not be read answers a table with the field absent, and one that
  -- has gone away answers nothing at all. Neither is an error worth a bar with a hole in it.
  local info = (focused and neotmux.pane.info(focused)) or {}

  local left = window_runs(info.command)
  local right = {}
  if info.command then
    append(right, module_runs(M.icons.command, M.colors.command, info.command))
  end
  if info.cwd then
    local room = row.w
      - segments.width(left)
      - segments.width(right)
      - module_chrome(M.icons.directory)
    local cwd = room > 0 and path.shorten(info.cwd, M.home, room) or nil
    if cwd then
      append(right, module_runs(M.icons.directory, M.colors.directory, cwd))
    end
  end

  local left_end = segments.draw(ctx, row.x, row.y, row.w, left)
  local width = segments.width(right)
  -- Right-aligned, and dropped whole rather than drawn over the window list: a bar that
  -- overlaps itself says less than one that says less.
  if row.x + row.w - width >= left_end then
    segments.draw(ctx, row.x + row.w - width, row.y, width, right)
  end
end)

return M

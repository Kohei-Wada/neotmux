# neotmux — one place for every plugin's colours: a theme and a style cascade

Date: 2026-09-28
Status: proposed
Scope: #34, colours only. Two runtime convention modules: `neotmux.theme`, the named colours,
and `neotmux.style`, the cascade that plugins read their colours from. Every shipped plugin
moves onto them, and `M.colors` goes.

## 1. Goal and non-goals

### Goal

Today 16 runtime files keep their own colour tables, 80 hex literals in all, and every one of
them is a Catppuccin Mocha colour under a local name. The same blue is `accent` in the status
bar, `mode` in the sidebar's bar, `key` in `C-b ?` and `spark` in `costs`. A user who wants a
different look edits 16 tables. A theme — the look swapped as one piece, which the herdr mode
idea needs — cannot exist.

After this:

- **The theme.** `neotmux.theme` names the colours. Swapping it recolours every plugin at once.
- **The style.** `neotmux.style` says which named colour each part of each plugin uses, as a
  cascade. The user can restyle all floats at once, one plugin, or one part of it.

### Non-goals

- Anything but colours. Borders, glyphs, padding and widths stay where they are (`M.width`,
  `M.rule`, `neotmux.box`'s corners).
- Selectors conditioned on state (`pane:focused`, `pane[env=production]`). That is #29.
- A theme other than Mocha, and the herdr mode itself. This makes both a small addition later.
- Compatibility with `M.colors`. It is removed, not kept beside the new thing, and the
  CHANGELOG says so.

## 2. The theme

```lua
local theme = require('neotmux.theme')
theme.colors.blue            --> '#89b4fa'
theme.set({ blue = '#7aa2f7', base = '#1a1b26' })   -- some of them, or all
theme.resolve('blue')        --> '#89b4fa'; a '#rrggbb' is itself; an unknown name is nil
```

- `theme.colors` holds the 26 Catppuccin Mocha names: `rosewater` `flamingo` `pink` `mauve`
  `red` `maroon` `peach` `yellow` `green` `teal` `sky` `sapphire` `blue` `lavender` `text`
  `subtext1` `subtext0` `overlay2` `overlay1` `overlay0` `surface2` `surface1` `surface0`
  `base` `mantle` `crust`. Every colour the runtime uses today is one of them.
- `theme.set(t)` replaces the named ones and leaves the rest. A theme may also add names of
  its own.

## 3. The style

```lua
local style = require('neotmux.style')

-- a plugin, at load: its parts and the colours they are by default
style.default('status', { bar = 'mantle', accent = 'blue', tab = 'surface0' })

-- the user, in init.lua, whenever it runs
style.set('*', { rule = 'surface2' })         -- every plugin's `rule`
style.set('float', { frame = 'crust' })       -- every float's frame
style.set('status', { accent = 'mauve' })     -- one plugin's part
style.set('status', { accent = '#ff8800' })   -- a hex is a colour too

-- a plugin, when it draws
local c = style.get('status')   --> { bar = '#181825', accent = '#cba6f7', ... }, resolved
```

- **Selectors are dotted names.** `get('float.prompt')` reads `*`, then `float`, then
  `float.prompt`, and a later one wins key by key.
- **The user always beats a default, whatever order they ran in.** The user's `init.lua` runs
  before the shipped layer loads a single plugin, so "the later wins" would let every shipped
  default override the user. There are two layers instead:
  - `default` is a plugin's own. It is lowest, and a second `default` for the same selector
    replaces the first, so a reload does not stack them.
  - `set` is the user's. It is always above every default, merged key by key.

  So `get` merges every default from `*` to the most specific selector, and then every set
  from `*` to the most specific. A user's `style.set('*', { rule = ... })` therefore beats a
  plugin's `default('stamps', { rule = ... })`, which is what "one place for every plugin"
  means.
- **Values are theme names or `#rrggbb`.** They are resolved at `get`, so `theme.set` and
  `style.set` take effect on the next frame, with no reload. A name the theme does not have
  resolves to nil, and a nil colour is the terminal's own default, as it is today.
- **Cheap to read every frame.** `get` caches each selector's merged, resolved table. The cache
  is dropped on any `default`, `set` or `theme.set`. The table returned is the cache's own; a
  plugin reads it and does not write to it.
- **`config-reload`** re-runs everything. Both modules are runtime modules and are loaded
  again, so their state starts empty and is rebuilt by the plugins and `init.lua` as they run.

## 4. Selectors the shipped plugins declare

| selector | file | parts (today's `M.colors` keys) |
|---|---|---|
| `status` | plugin/tmux/status | bar accent on_accent tab tab_text named_text right_text dim |
| `sidebar` | plugin/herdr/sidebar | text subtext overlay branch here header session current_session window current_window pane focused_pane detail rule working blocked done idle unknown selection bar |
| `sidebar.bar` | plugin/herdr/sidebar/spaces | mode on text (today `M.bar_colors`) |
| `sessions` | plugin/tmux/sessions | name current working blocked idle doing |
| `notify` | plugin/notify | frame tag info warn error |
| `exits` | plugin/exits | ok failed running time rule |
| `stamps` | plugin/stamps | time rule slow stuck |
| `float.list` | neotmux/list | frame text cursor caret hint |
| `float.prompt` | neotmux/prompt | frame text caret hint |
| `float.box` | neotmux/box | frame line title |
| `palette` | plugin/palette | name match desc source |
| `copy` | plugin/tmux/copy | selection cursor cursor_text |
| `footer` | plugin/footer | bar text |
| `hints` | neotmux/hints | bg fg |
| `prefix` | plugin/tmux/prefix | key name desc (today `KEY_COLOURS`) |
| `costs` | plugin/costs | source number label spark |

`notify` draws a float, but its parts are not a list's or a prompt's, so it stays `notify`.
`sidebar`'s sections (`spaces`, `agents`, `sessions`) read `style.get('sidebar')`, since they
draw in the sidebar's colours.

Every default is today's hex spelled as its Mocha name. What a plugin draws does not change by
one colour until someone sets something.

## 5. Testing

- `neotmux-lua/tests/style.rs` covers:
  - defaults below sets, whatever the order they ran in
  - specificity, `*` < `float` < `float.list`
  - `default` replacing its own earlier call
  - names resolved through the theme, and a hex passed through as itself
  - `theme.set` taking effect on the next `get`
  - an unknown name resolving to nil
- Every existing plugin test passes unchanged: the colours drawn are the same hex. Tests
  written against `M.colors` (stamps' `colors`, exits' `OK` and `FAILED`, sidebar's colours)
  are only moved onto `style.get`, if they read the table.
- By hand: `style.set('*', { rule = 'red' })` and `theme.set({ blue = '#ff0000' })` in
  `init.lua`. The rules turn red, the status bar's current tab and every other blue turn red,
  and nothing else moves.

## 6. Docs

- A "Styling" section in `docs/plugins.md`: the theme, the cascade, the table above, and how a
  third-party plugin declares its own selector.
- Every "Changing what X looks like" subsection moves from `M.colors` to `style.set`.
- The CHANGELOG entry names the break: `require('plugin.x').colors` is gone, and here is the
  `style.set` that replaces it.

# Theme and style cascade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every shipped plugin reads its colours from `neotmux.style`, whose values are names in `neotmux.theme`; `M.colors` and the other per-file colour tables are gone.

**Architecture:** `neotmux.theme` holds the 26 Catppuccin Mocha names and resolves a name or a `#rrggbb`. `neotmux.style` keeps two layers per dotted selector — `default` (a plugin's, lowest, replaced on a second call) and `set` (the user's, merged, always above every default) — and `get(selector)` merges `*` → … → the selector, defaults first then sets, resolves through the theme, and caches until anything changes.

**Tech Stack:** Lua runtime (`runtime/lua/`), Rust integration tests in `neotmux-lua/tests`.

**Spec:** `docs/superpowers/specs/2026-09-28-style-and-theme-design.md`

## Global Constraints

- Work in `.worktrees/style` (branch `style`). Never touch the main checkout. Never use bare `git stash`.
- Colours only. Every default is today's hex spelled as its Mocha name; nothing drawn changes by one colour until a user sets something.
- `M.colors`, `M.bar_colors` (spaces) and `KEY_COLOURS` (prefix) are removed outright — no compatibility shim.
- Selectors and parts are exactly spec §4's table.
- The user's `style.set` beats every `style.default`, whatever order they ran in.
- A value is a theme name or `#rrggbb`; an unknown name resolves to nil (the terminal's default colour).
- One-line comments only where the why is non-obvious; match the surrounding prose style.
- `make check` passes at the end of every task.
- Commits: lowercase sentence of what is now true, ending with
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_019EYxiTuCeWDUwaWAg81FbD
  ```

## Mocha names (hex → name), for every migration below

```
#f5e0dc rosewater  #f2cdcd flamingo  #f5c2e7 pink      #cba6f7 mauve    #f38ba8 red
#eba0ac maroon     #fab387 peach     #f9e2af yellow    #a6e3a1 green    #94e2d5 teal
#89dceb sky        #74c7ec sapphire  #89b4fa blue      #b4befe lavender #cdd6f4 text
#bac2de subtext1   #a6adc8 subtext0  #9399b2 overlay2  #7f849c overlay1 #6c7086 overlay0
#585b70 surface2   #45475a surface1  #313244 surface0  #1e1e2e base     #181825 mantle
#11111b crust
```

## Review Focus

1. A user's `style.set` in `init.lua` runs before any plugin's `style.default` — it must still win. Task 1.
2. `config-reload` with a user `style.set` present: after the reload the user's colour is still applied (both modules reload and are rebuilt in the same order). Task 1 test via `sub.reload()` if the harness supports it, else by hand in Task 4.
3. A plugin that caches `style.get(...)` at load instead of per draw would ignore later `set`s — every migrated plugin reads `style.get` where it draws. Tasks 2–3.
4. Sidebar sections (spaces, agents, sessions) read the sidebar's colours through what they are handed — after the move they must read `style.get('sidebar')` so a user override reaches them. Task 3.
5. An unknown name (a user typo) must not raise every tick — it resolves to nil. Task 1.

---

### Task 1: `neotmux.theme` and `neotmux.style`

**Files:**
- Create: `runtime/lua/neotmux/theme.lua`, `runtime/lua/neotmux/style.lua`
- Test: `neotmux-lua/tests/style.rs` (new)

**Interfaces:**
- Produces: `theme.colors`, `theme.set(t)`, `theme.resolve(v) -> '#rrggbb' | nil`, `theme.generation`; `style.default(selector, parts)`, `style.set(selector, parts)`, `style.get(selector) -> table`.

- [ ] **Step 1: Write the failing tests** — `neotmux-lua/tests/style.rs`:

```rust
//! `neotmux.theme` and `neotmux.style`: named colours, and the cascade every plugin reads its
//! colours from.

mod support;

use support::{built, keys, logged};

/// A key that logs `style.get(selector)[part]` for each pair, after the init has run.
fn probe(pairs: &[(&str, &str)]) -> String {
    let lines: String = pairs
        .iter()
        .map(|(selector, part)| {
            format!(
                "neotmux.log.info('{selector}.{part}=' .. tostring(require('neotmux.style').get('{selector}').{part}))\n"
            )
        })
        .collect();
    format!("neotmux.on_key(function(key) if key ~= 'P' then return false end\n{lines}return true end)\n")
}

#[test]
fn a_users_set_beats_a_default_whatever_order_they_ran_in() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.set('status', {{ accent = 'mauve' }})
         style.default('status', {{ accent = 'blue', bar = 'mantle' }})
         {}",
        probe(&[("status", "accent"), ("status", "bar")])
    ));
    keys(&sub, &["P"]);
    assert_eq!(logged(&sub), vec!["status.accent=#cba6f7", "status.bar=#181825"]);
}

#[test]
fn a_more_specific_selector_wins_and_a_users_general_one_beats_a_specific_default() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.default('float', {{ frame = 'crust', text = 'text' }})
         style.default('float.list', {{ frame = 'mantle' }})
         style.set('*', {{ text = 'red' }})
         {}",
        probe(&[("float.list", "frame"), ("float.list", "text"), ("float", "frame")])
    ));
    keys(&sub, &["P"]);
    assert_eq!(
        logged(&sub),
        vec!["float.list.frame=#181825", "float.list.text=#f38ba8", "float.frame=#11111b"]
    );
}

#[test]
fn a_second_default_replaces_the_first() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.default('stamps', {{ time = 'blue', rule = 'red' }})
         style.default('stamps', {{ time = 'green' }})
         {}",
        probe(&[("stamps", "time"), ("stamps", "rule")])
    ));
    keys(&sub, &["P"]);
    assert_eq!(logged(&sub), vec!["stamps.time=#a6e3a1", "stamps.rule=nil"]);
}

#[test]
fn a_hex_is_itself_and_an_unknown_name_is_nil() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.set('x', {{ a = '#ff8800', b = 'no-such-colour' }})
         {}",
        probe(&[("x", "a"), ("x", "b")])
    ));
    keys(&sub, &["P"]);
    assert_eq!(logged(&sub), vec!["x.a=#ff8800", "x.b=nil"]);
}

/// The theme is read when a colour is asked for, so swapping it recolours the next frame.
#[test]
fn a_theme_change_reaches_the_next_get() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.default('status', {{ accent = 'blue' }})
         neotmux.on_key(function(key)
           if key ~= 'T' then return false end
           require('neotmux.theme').set({{ blue = '#010203' }})
           return true
         end)
         {}",
        probe(&[("status", "accent")])
    ));
    keys(&sub, &["P", "T", "P"]);
    assert_eq!(logged(&sub), vec!["status.accent=#89b4fa", "status.accent=#010203"]);
}
```

- [ ] **Step 2: Run to see it fail** — `cargo test -p neotmux-lua --test style`. Expected: FAIL (`module 'neotmux.style' not found`).

- [ ] **Step 3: Implement.**

`runtime/lua/neotmux/theme.lua`:

```lua
-- The colours, by name: what every plugin's style is written in, so that swapping these
-- recolours all of them at once. Catppuccin Mocha, which every shipped colour is one of.
--
-- A convention, not a plugin: it registers nothing.
--
--   local theme = require('neotmux.theme')
--   theme.set({ blue = '#7aa2f7', base = '#1a1b26' })   -- some of them, or all
local M = {
  colors = {
    rosewater = "#f5e0dc",
    flamingo = "#f2cdcd",
    pink = "#f5c2e7",
    mauve = "#cba6f7",
    red = "#f38ba8",
    maroon = "#eba0ac",
    peach = "#fab387",
    yellow = "#f9e2af",
    green = "#a6e3a1",
    teal = "#94e2d5",
    sky = "#89dceb",
    sapphire = "#74c7ec",
    blue = "#89b4fa",
    lavender = "#b4befe",
    text = "#cdd6f4",
    subtext1 = "#bac2de",
    subtext0 = "#a6adc8",
    overlay2 = "#9399b2",
    overlay1 = "#7f849c",
    overlay0 = "#6c7086",
    surface2 = "#585b70",
    surface1 = "#45475a",
    surface0 = "#313244",
    base = "#1e1e2e",
    mantle = "#181825",
    crust = "#11111b",
  },
  -- Moved by every `set`, so a cache of resolved colours knows to let go of them.
  generation = 0,
}

-- Replaces the colours named in `t` and leaves the rest; a name of its own is added.
function M.set(t)
  for name, hex in pairs(t) do
    M.colors[name] = hex
  end
  M.generation = M.generation + 1
end

-- A name's colour, a `#rrggbb` as itself, or nil for a name the theme does not have — which
-- a draw takes as the terminal's own colour, so a typo shows as a plain colour and not an error
-- on every tick.
function M.resolve(value)
  if type(value) ~= "string" then
    return nil
  end
  if value:sub(1, 1) == "#" then
    return value
  end
  return M.colors[value]
end

return M
```

`runtime/lua/neotmux/style.lua`:

```lua
-- Which colour each part of each plugin is: a cascade over dotted selectors, in the theme's
-- names or in `#rrggbb`.
--
-- A convention, not a plugin: it registers nothing.
--
--   local style = require('neotmux.style')
--   style.default('status', { bar = 'mantle', accent = 'blue' })   -- a plugin, at load
--   style.set('float', { frame = 'crust' })                        -- the user, in init.lua
--   local c = style.get('status')                                  -- a plugin, when it draws
--
-- `get('float.list')` reads `*`, then `float`, then `float.list`, a later one winning key by key.
-- Every default comes before every set: the user's `init.lua` runs before the shipped layer
-- loads a single plugin, so if the later call won, every shipped default would override the
-- user. A plugin's `default` for a selector replaces its earlier one, so a reload does not stack
-- them; the user's `set`s merge.
local theme = require("neotmux.theme")

local M = {}

local defaults, sets = {}, {}
local cache, seen = {}, theme.generation

local function copy_into(into, from)
  for key, value in pairs(from) do
    into[key] = value
  end
end

function M.default(selector, parts)
  local own = {}
  copy_into(own, parts)
  defaults[selector] = own
  cache = {}
end

function M.set(selector, parts)
  sets[selector] = sets[selector] or {}
  copy_into(sets[selector], parts)
  cache = {}
end

-- `*`, then each dotted prefix of `selector`, ending with `selector` itself.
local function chain(selector)
  local out = { "*" }
  if selector == "*" then
    return out
  end
  local from = 1
  while true do
    local dot = selector:find(".", from, true)
    out[#out + 1] = dot and selector:sub(1, dot - 1) or selector
    if not dot then
      return out
    end
    from = dot + 1
  end
end

-- The merged, resolved colours for `selector`. Cached until anything changes, so reading it
-- where you draw, every frame, is a table lookup. The table is the cache's: read it, do not
-- write to it.
function M.get(selector)
  if seen ~= theme.generation then
    cache, seen = {}, theme.generation
  end
  local hit = cache[selector]
  if hit then
    return hit
  end
  local levels = chain(selector)
  local merged = {}
  for _, level in ipairs(levels) do
    copy_into(merged, defaults[level] or {})
  end
  for _, level in ipairs(levels) do
    copy_into(merged, sets[level] or {})
  end
  local resolved = {}
  for key, value in pairs(merged) do
    resolved[key] = theme.resolve(value)
  end
  cache[selector] = resolved
  return resolved
end

return M
```

- [ ] **Step 4: Run to see it pass** — `cargo test -p neotmux-lua --test style && stylua --check runtime && luacheck -q runtime`.
- [ ] **Step 5: Commit** — `git add runtime/lua/neotmux/theme.lua runtime/lua/neotmux/style.lua neotmux-lua/tests/style.rs && git commit -m "neotmux.theme names the colours and neotmux.style cascades them over dotted selectors, the user's above every default"`

---

### Task 2: Every plugin but the sidebar reads `neotmux.style`

**Files (modify):** `runtime/lua/plugin/tmux/status.lua`, `runtime/lua/plugin/tmux/sessions.lua`, `runtime/lua/plugin/notify.lua`, `runtime/lua/plugin/exits.lua`, `runtime/lua/plugin/stamps.lua`, `runtime/lua/neotmux/list.lua`, `runtime/lua/neotmux/prompt.lua`, `runtime/lua/neotmux/box.lua`, `runtime/lua/plugin/palette.lua`, `runtime/lua/plugin/tmux/copy.lua`, `runtime/lua/plugin/footer.lua`, `runtime/lua/neotmux/hints.lua`, `runtime/lua/plugin/tmux/prefix.lua`, `runtime/lua/plugin/costs.lua`; test `neotmux-lua/tests/status.rs:265`.

**Interfaces:** Consumes Task 1. Selectors and parts exactly as spec §4 (`status`, `sessions`, `notify`, `exits`, `stamps`, `float.list`, `float.prompt`, `float.box`, `palette`, `copy`, `footer`, `hints`, `prefix`, `costs`).

**The rule, for every file:**
1. Replace the colour table (`M.colors = { ... }`, a `colors = { ... }` field inside `M`, `KEY_COLOURS`) with a `style.default('<selector>', { <part> = '<mocha name>', ... })` call at load — same parts, each hex written as its Mocha name from the table above.
2. Where the file draws, read `local c = style.get('<selector>')` (inside the draw/tick function, never at load) and use `c.<part>` where it used `M.colors.<part>`.
3. Keep each existing comment on a colour, moved onto its line in the `style.default` call. Update the header comment's example if it shows `M.colors` (`require('plugin.x').colors.y = ...` becomes `require('neotmux.style').set('<selector>', { y = ... })`).
4. A file whose colours another file reads through it (`sessions.lua:18` mentions `require('neotmux.list').colors`) — change the reader to `style.get('float.list')`.
5. `grep -n "colors\|COLOURS" <file>` afterwards finds no colour table and no `M.colors` read.

- [ ] **Step 1:** `neotmux-lua/tests/status.rs:265` — change the test's init to `require('neotmux.style').set('status', { accent = '#ff0000' })` (same expectation). Run `cargo test -p neotmux-lua --test status` — expect it to FAIL (the status bar does not read the style yet).
- [ ] **Step 2:** Migrate the 14 files by the rule.
- [ ] **Step 3:** `grep -rnE '"#[0-9a-fA-F]{6}"' runtime/lua | grep -v neotmux/theme.lua` must list only the sidebar family (`plugin/herdr/sidebar.lua`, `plugin/herdr/sidebar/spaces.lua`). Then `make check` — expect PASS (every drawn colour is the same hex).
- [ ] **Step 4: Commit** — `git commit -m "every shipped plugin but the sidebar reads its colours from neotmux.style, in the theme's names"`

---

### Task 3: The sidebar and its sections read `neotmux.style`

**Files (modify):** `runtime/lua/plugin/herdr/sidebar.lua`, `runtime/lua/plugin/herdr/sidebar/spaces.lua`, `runtime/lua/plugin/herdr/sidebar/agents.lua`, `runtime/lua/plugin/herdr/sidebar/sessions.lua`; test `neotmux-lua/tests/sidebar.rs:1091`.

**Interfaces:** Selectors `sidebar` (every part in spec §4, including `bar`) and `sidebar.bar` (`mode`, `on`, `text` — today spaces' `M.bar_colors`).

- `sidebar.lua`: its colour table becomes `style.default('sidebar', {...})`; its own draws read `style.get('sidebar')`.
- The sections read `sidebar.colors.<part>` 33 times through the module they are handed. Replace each with a read of `style.get('sidebar')` taken once per build/fit/draw function (`local c = style.get('sidebar')`), and remove `M.colors` from `sidebar.lua` — nothing may keep reading a `colors` field.
- `spaces.lua`'s `M.bar_colors` becomes `style.default('sidebar.bar', {...})`, read with `style.get('sidebar.bar')`.
- Update header comments that say "coloured through `require('plugin.herdr.sidebar').colors`" to name the `sidebar` selector.

- [ ] **Step 1:** `neotmux-lua/tests/sidebar.rs:1091` → `require('neotmux.style').set('sidebar', { bar = '#181825' })`. Run the sidebar tests — expect this one to fail or to still pass for the wrong reason; note which in the report.
- [ ] **Step 2:** Migrate.
- [ ] **Step 3:** `grep -rnE '"#[0-9a-fA-F]{6}"' runtime/lua | grep -v neotmux/theme.lua` is empty; `grep -rn "\.colors\b\|bar_colors" runtime/lua` finds nothing but comments naming the old spelling in the CHANGELOG sense (none should remain). `make check` passes.
- [ ] **Step 4: Commit** — `git commit -m "the sidebar and its sections read their colours from neotmux.style"`

---

### Task 4: Docs, changelog, the whole check

- [ ] `docs/plugins.md`: a "### Styling" section (after "### The box convention" or beside "The shipped defaults") with the spec's §2–§3 code, the §4 table, and how a third-party plugin declares its selector with `style.default` and reads it with `style.get` where it draws. Every "Changing what X looks like" subsection and every `require('plugin.…').colors` example moves to `style.set`. Search with `grep -n "colors" docs/plugins.md`.
- [ ] `CHANGELOG.md` entry at the top of Unreleased:

```markdown
### One place for every plugin's colours

Every shipped plugin now reads its colours from `neotmux.style`, and every colour is a name in
`neotmux.theme` — the 26 Catppuccin Mocha colours, which is what they all were. Restyle every
float, one plugin, or one part of it with `require('neotmux.style').set('float', { frame =
'crust' })`; recolour everything at once with `require('neotmux.theme').set({ blue =
'#7aa2f7' })`. **`require('plugin.x').colors` is gone**: `require('plugin.tmux.status').colors.
accent = '#ff0000'` is now `require('neotmux.style').set('status', { accent = '#ff0000' })`, and
`docs/plugins.md` lists every selector and its parts.
```

- [ ] `make check` passes. Commit — `git commit -m "docs for neotmux.theme and neotmux.style, and every look is changed through them"`.
- [ ] **(controller) Real screen:** `init.lua` with `require('neotmux.style').set('*', { rule = 'red' })` and `require('neotmux.theme').set({ blue = '#ff0000' })`; with the sidebar on and `C-b t` shown, the rules and every blue (status bar's current tab, sidebar's mode) are red, nothing else changed; `config-reload` keeps them red.

---

### Task 5: Twelve themes, and a `theme` command in the palette

Added at the user's request (2026-09-29): pick a theme from the palette; to keep it, the user
writes it in `init.lua` — neotmux saves nothing. Themes: the four Catppuccin flavours, and the
popular ones the user asked for.

**Files:** create `runtime/lua/neotmux/themes/<name>.lua` (one per theme), create
`runtime/lua/plugin/theme.lua`; modify `runtime/lua/neotmux/theme.lua`,
`runtime/lua/neotmux/defaults.lua`, `runtime/init.lua` (load list), `docs/plugins.md`,
`CHANGELOG.md`; test `neotmux-lua/tests/theme.rs` (new).

**Themes and their sources** — every hex comes from the source, fetched, never from memory:

| name | file | source |
|---|---|---|
| `catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `catppuccin-mocha` | `themes/catppuccin-*.lua` | https://raw.githubusercontent.com/catppuccin/palette/main/palette.json (`<flavour>.colors.<name>.hex`) |
| `tokyonight-night`, `tokyonight-storm` | | folke/tokyonight.nvim `extras/lua/tokyonight_night.lua` / `_storm.lua` (or `lua/tokyonight/colors/{night,storm}.lua` + `colors/storm.lua`) |
| `gruvbox-dark` | | morhetz/gruvbox `colors/gruvbox.vim` (dark, medium contrast) |
| `nord` | | https://www.nordtheme.com/docs/colors-and-palettes (nord0–nord15) |
| `dracula` | | https://draculatheme.com/spec |
| `rose-pine` | | rose-pine/palette (main variant) |
| `kanagawa` | | rebelot/kanagawa.nvim `lua/kanagawa/colors.lua` (wave) |
| `onedark` | | navarasu/onedark.nvim `lua/onedark/palette.lua` (dark) |
| `everforest-dark` | | sainnhe/everforest `autoload/everforest.vim` (dark, medium) |

Mocha's file must equal today's `theme.colors` exactly (it is where those values now live;
`theme.colors` starts as a copy of it).

**Mapping a non-Catppuccin theme onto the 26 names.** Every theme file sets all 26 names, with
a one-line comment per name that is not the source's own name for it, saying what it is in the
source:
- neutrals, darkest to lightest: `crust` < `mantle` < `base` (the editor background) <
  `surface0` < `surface1` < `surface2` (selection/line highlight shades) < `overlay0` <
  `overlay1` < `overlay2` (comments, inactive) < `subtext0` < `subtext1` < `text` (foreground).
  Where the source has fewer shades, reuse the nearest and say so.
- accents: `red`, `maroon` (a second red or the red), `peach` (orange), `yellow`, `green`,
  `teal`, `sky`, `sapphire` (cyans/blue-cyans; reuse the nearest), `blue`, `lavender` (a light
  blue/violet), `mauve` (purple/magenta), `pink`, `flamingo`, `rosewater` (light pinks; reuse
  the nearest).

**Interfaces:**
- `theme.names` — the themes in the order above, `catppuccin-*` first.
- `theme.use(name)` — `theme.set(require('neotmux.themes.' .. name))`, `theme.current = name`;
  an unknown name raises `unknown theme '<name>'`. `theme.current` starts as `'catppuccin-mocha'`.
- `plugin.theme` — on by default (`defaults.theme = true`, loaded from `runtime/init.lua`'s list):
  the command `theme` (desc: "pick a theme; add require('neotmux.theme').use('<name>') to init.lua
  to keep it") opens a `neotmux.list` of `theme.names`, the current one marked; Enter calls
  `theme.use(name)` and `neotmux.notify('info', 'theme ' .. name .. " — to keep it, add
  require('neotmux.theme').use('" .. name .. "') to init.lua")`. No prefix key.

- [ ] Tests first (`neotmux-lua/tests/theme.rs`): every name in `theme.names` loads and sets all
  26 names to a `#rrggbb`; `theme.use('catppuccin-latte')` changes `style.get('status').accent`
  to Latte's blue from the fetched palette; `theme.use('nope')` raises; the `theme` command opens a
  list (drive it as `tests/list.rs` does) and Enter on a theme applies it and notifies.
- [ ] Implement; `make check`; commit `"twelve themes in neotmux.themes, and a theme command that
  picks one"`.
- [ ] Docs: the Styling section lists the themes, `theme.use`, and the `theme` command; the
  shipped-defaults table gets a `theme` row; the CHANGELOG style entry gains a line.

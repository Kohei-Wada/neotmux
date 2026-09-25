# neotmux — a sidebar that does what herdr's does

Date: 2026-09-25
Status: proposed
Scope: the sidebar's default sections, a `done` state, palette providers, and the keys that move
between sessions and agents — so a herdr user finds what they used and nothing to relearn.

## 1. Goal and non-goals

### Goal

The user is moving from herdr (herdrdev/herdr) and wants the sidebar to be herdr's. What herdr's
is, was read from its source at `8d95e9bd` (2026-09-24): `src/client/shell/sidebar.rs`,
`agent_sidebar.rs`, `src/config/sidebar.rs`, `src/workspace/aggregate.rs`, and its docs. herdr's
levels are session → workspace ("space") → tab → pane; a herdr **space** is a neotmux **session**
and a herdr **tab** a neotmux **window**.

What the user gets:

1. **The two sections herdr has.** A `spaces` section — every session, two rows each: its state
   and name, then its git branch and how far it is ahead of and behind its upstream — and under
   it the `agents` section, in herdr's row shape.
2. **herdr's states and colours**, including **`done`**: an agent that finished while you were
   not looking at it, until you look.
3. **herdr's keys.** `C-b w` moves a cursor over the sessions and Enter goes there; `C-b g` is a
   picker of every agent that jumps to the one you choose.
4. **A palette you can add to.** Providers, so sessions and agents are in the palette beside the
   commands, and a plugin can add its own.

### Non-goals

- **The mouse.** neotmux is keyboard only (its positioning, 2026-09-24). No clicking, dragging or
  resizing by hand.
- **herdr's compact rail** (four columns of numbers and icons when folded), **grouped git
  worktrees**, **remote machines** and **metadata tokens reported by hooks**. Each is its own
  change if it is wanted.
- **Changing `neotmux.state`.** `done` is not a detector's answer; it is what the sidebar knows
  about what you have seen (§3). The chime and the watcher keep reading `state.of` as they do.
- **Throwing the tree away.** `plugin.sidebar.sessions` stays, unchanged, one assignment away
  (§2.1).

## 2. Sections

### 2.1 The default list

```lua
require('plugin.sidebar').sections = {
  require('plugin.sidebar.spaces'),   -- new, herdr's top half
  require('plugin.sidebar.agents'),   -- reshaped to herdr's rows
}
```

The tree is still there for anyone who wants it back, above or in place of `spaces`:

```lua
table.insert(require('plugin.sidebar').sections, 1, require('plugin.sidebar.sessions'))
```

### 2.2 The container

Two changes, both herdr's:

- **Every titled section draws its header, the first one too.** herdr heads the top section
  `spaces`. The tree has no title, so it still draws none.
- **A rule down the right edge**, `│` in the rule colour, in the band's last column. Rows are
  drawn one column narrower to leave it.

### 2.3 `plugin.sidebar.spaces`

- One entry per session, in `session.list()` order.
- Default rows, herdr's: `{ 'state_icon', 'name' }` and `{ 'branch', 'git_status' }`, the second
  indented under the first. A row whose tokens are all empty is dropped, as before, so a session
  outside git is one row.
- `state_icon` is the session's rollup: the highest of its panes by herdr's order, blocked >
  done > working > idle.
- `name` is drawn in `colors.current_session` for the session this tick is for, as the tree draws it.
- `branch` and `git_status` come from the cwd of the session's focused pane (`pane.info().cwd`),
  asked of git by `neotmux.job` — the sandbox has no `io`:
  `git -C <cwd> rev-parse --abbrev-ref HEAD` and
  `git -C <cwd> rev-list --left-right --count @{upstream}...HEAD`.
  Answers are cached per cwd and asked again at most every `M.refresh` ms (default 5000), with
  at most one pair of jobs in flight per cwd. `git_status` is `↑N` (green) and `↓N` (red), each
  only when nonzero; no upstream is no status.
- **The navigate cursor** (§5.1) is drawn as a `colors.selection` background on the entry's rows.

### 2.4 `plugin.sidebar.agents`

- Default rows, herdr's: `{ 'state_icon', 'session', 'window' }` and `{ 'agent' }`.
  `window` is the window's name, or its number when it has none, and is left out when the
  session has one window. The existing `location`, `command` and `age` tokens stay available.
- **Order** is `M.sort`: `'grouped'` (the default, herdr's) is session, window, pane order;
  `'priority'` is blocked > done > working > idle, then session order — what the section does
  today.
- The header carries the sort on its right, as herdr's does: `─ agents ──── grouped`.

## 3. `done`: finished, not yet seen

`neotmux.seen` (new, `runtime/lua/neotmux/seen.lua`), shared by the sections and the palette:

```lua
local seen = require('neotmux.seen')
seen.of(pane, state)   -- state, or 'done' where the pane went idle while nobody was looking
seen.look()            -- called once per tick; marks what is being looked at
```

- A pane is **looked at** when it is in the current window of a session a tick is running for.
- A pane whose state goes from `working` or `blocked` to `idle` **while not looked at** is
  unseen. `seen.of` answers `'done'` for it until it is next looked at, and `'idle'` after.
- It is kept per pane and dropped when a look no longer finds the pane.

herdr derives `done` the same way (`src/app/api_helpers.rs:96-105`, `src/workspace.rs:451-459`);
herdr also asks whether the outer terminal has focus, which neotmux does not know, so "looked
at" here is "on screen in an attached session".

## 4. Colours and icons

herdr's defaults (`src/client/shell.rs:178-237`), as `plugin.sidebar`'s:

| state | icon | colour |
|---|---|---|
| working | `●` | yellow `#f9e2af` |
| blocked | `●` | red `#f38ba8` |
| done | `●` | teal `#94e2d5` |
| idle | `○` | green `#a6e3a1` |

`sidebar.icons` holds them, so the sidebar no longer draws `neotmux.state.icons`; `state.icons`
keeps what the status bar and anything else draws. herdr's `symbols` set (`◐ × ✓ ○`) is one
assignment: `require('plugin.sidebar').icons = { working = '◐', blocked = '×', done = '✓', idle = '○' }`.

## 5. Keys

### 5.1 `C-b w`: navigate

herdr's navigate mode (`src/client/shell/input.rs:643-822`), on the `spaces` section:

- `C-b w` raises a `navigate` mode (`neotmux.mode`), and the cursor starts on the current
  session. The sidebar unfolds for the duration if it was folded.
- `j`/`k` (and the arrows) move it, wrapping; nothing switches yet.
- `Enter` switches to the session under it and leaves the mode.
- `1`–`9` switch to that session and leave.
- `Esc` or `C-b` leaves.

### 5.2 `C-b g`: go to an agent

The palette, opened on its agents provider (§6): every agent in every session, one row each —
state icon, `session:window`, the agent — fuzzy-filtered as you type, Enter goes there. Typing
`blocked` or `done` narrows by state, since the state is part of the row's text.

**Going there** is a new command, `agent-focus`, used by the provider: it shows the window
holding the pane and focuses the pane in that pane's session, then switches to the session. Two
more commands, `agent-next` and `agent-prev`, step through the agents in `M.sort` order; they
are in the palette and bound to nothing, as herdr's are.

## 6. Palette providers

```lua
local palette = require('plugin.palette')
palette.providers = {
  { prefix = '',  name = 'commands', items = function(query) ... end, run = function(item) ... end },
  { prefix = '@', name = 'sessions', ... },
  { prefix = '#', name = 'agents', ... },
}
```

- The query's first character picks the provider: `@` sessions, `#` agents, anything else the
  commands. The prefix is taken off before the rest is matched.
- An item is `{ name = ..., desc = ..., source = ... }` and whatever `run` needs. Every provider
  is ranked by the same fuzzy scorer.
- `palette.open(prefix)` opens the palette with that prefix already typed; `C-b g` is
  `palette.open('#')`.
- A provider that raises is left out of that query and reported once, like a section.

## 7. What has to be true first

**Focusing a pane in another session.** `agent-focus` depends on ops raised inside
`session.with(name, fn)` being applied to *that* session. The docs say `with` rebinds reads and
say nothing of ops. The first task proves it with a host-less test; if it does not hold, the
change stops and the host gets a `neotmux.session.focus(name, pane)` of its own before anything
else is built on it.

## 8. Testing

- Host-less (`neotmux-lua/tests/sidebar.rs`, `palette.rs`, a new `seen.rs`):
  - spaces: one entry per session, the rollup order, the branch row from a fake job runner, the
    row dropped outside git, the cursor under navigate;
  - agents: herdr's rows, both sorts, the header label;
  - seen: working→idle unwatched is done, looked at is idle again, watched transitions are never
    done;
  - palette: each prefix picks its provider, a raising provider is skipped once;
  - the op-in-`with` property of §7.
- End to end (`neotmux/tests/fixtures.rs`): `C-b w` then `j` and Enter switches session; `C-b g`
  then Enter focuses the chosen agent's pane.
- By eye: a real screen with several sessions and a real `claude`, captured and read.

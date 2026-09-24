# neotmux — knowing what an agent is doing

Date: 2026-09-24
Status: proposed
Scope: telling whether the agent in a pane is working, waiting for you, or done — well enough
to act on — and making a sound when that changes.

## 1. Goal and non-goals

### Goal

`neotmux.state` answers `working`, `blocked` or `idle` for a pane by matching its last five
non-blank lines against eight patterns like `[y/N]` and `Do you want to`, and by calling
anything that printed in the last two seconds "working". That is enough to tint a row in a
sidebar and no more. It says `blocked` for a pane showing this repository's own source, and it
says `idle` for an agent that is thinking quietly.

An agent is either getting on with it, waiting for you, or done, and **which of those it is, is
the thing you open a sidebar to find out** — so the guess has to be worth trusting. This
replaces the guess with a real detector for Claude Code, and adds the thing a good detector
makes possible: a sound when a pane starts waiting for you, so you can stop watching it.

### Non-goals

- **Other agents.** herdr ships more than twenty manifests. This spec ports one, and leaves a
  table for the next.
- **Rewriting the sidebar.** `plugin.sidebar` already draws `state.of`; it gets a better answer
  and nothing else changes.
- **Finding our own sounds.** The two herdr ships are the two this needs, and they come with it.
- **A general regex engine.** The rules are hand-written Lua, not a ported rule format.

### Where this comes from

The rules are derived from herdr's `src/detect/manifests/claude.toml` and the hold in
`src/pane/agent_detection.rs`, and the two sounds are its `assets/sounds/{done,request}.mp3`
(herdrdev/herdr, Apache-2.0). What is taken is the knowledge of *which shapes on a Claude Code
screen mean what* — hard-won, and not ours to rediscover badly. The engine underneath is this
project's own. The attribution goes in `neotmux/detect.lua`'s header, beside the sounds, and in
`NOTICE`, which this spec adds because the repository is dual MIT/Apache-2.0 and has no NOTICE
file yet.

One thing the attribution cannot settle: herdr carries no NOTICE and no asset credits, so
whether those two mp3s are herdr's own work or came from a sound library with its own terms is
not knowable from the repository. Its Apache-2.0 licence covers them on its face, and that is
the assumption this is built on. If it ever matters, the fix is two files.

## 2. What the host has to add

Two small things, both of which the Lua layer cannot get any other way.

### 2.1 A pane's title

Claude Code writes its state into the terminal title: a braille spinner (`⠋`, `⠙`, …) or a
half-circle while it is working, `✳` when it is not. herdr's highest-priority rule reads it,
because a title is a statement the program made about itself rather than a guess about pixels —
it survives scrolling, resizing, a screen full of code, and a user typing a question that looks
like a prompt.

`neotmux-term` parses OSC 0 and OSC 2 today and throws them away. It keeps the last one
instead, and Lua reads it:

```lua
neotmux.pane.title(id)   --> the last title the pane set, or nil
```

Not `pane.info(id).title`: `PaneInfo` is what `/proc` says about a pane's process, cached with
an age and published per frame, and a title is what the program said, which is a different
kind of fact reached the way every other fact about a terminal is — through `PaneRead`,
directly, with nothing on the wire.

Nothing else changes: the title is not drawn, not put in a status bar, not used by the host.

### 2.2 A monotonic clock in milliseconds

The Lua layer's only clock is `os.time`, which counts whole seconds. Every timing this spec
needs is under a second, and `neotmux.state`'s existing two-second "working" window is coarse
for the same reason.

```lua
neotmux.now()   --> milliseconds, monotonic, from an arbitrary zero
```

Only differences mean anything. It is the same `Instant` the frame already holds, so a
callback and the tick around it see one value.

## 3. `neotmux.detect`

A new convention module: the rules, the regions they read, and the hold that keeps a flicker
from becoming a notification.

```lua
local detect = require('neotmux.detect')

detect.of(pane)          --> 'working' | 'blocked' | 'idle' | nil
detect.agents            --  the rulesets, by name; `claude` is the one that ships
detect.settle            --  700, the milliseconds a working→idle fall is held
```

### 3.1 A view of the pane

A rule reads a *view*, built once per pane per look and handed to every rule:

| field | what it is |
|---|---|
| `title` | the pane's title, or `''` |
| `lines` | the pane's non-blank lines, in order |
| `tail(n)` | the last `n` non-blank lines |
| `after_rule()` | the lines after the last horizontal rule (`───`), or all of them |
| `prompt_box()` | the lines inside the last box drawn with `│`, or none |

Those are herdr's regions, named for what they are rather than for the syntax it used. Every
one is a slice of `lines`, computed lazily, so a rule that never asks costs nothing.

### 3.2 A rule

```lua
{
  id = 'live_turn_working',
  state = 'working',
  priority = 970,
  visible = 'working',             -- optional; see the hold
  match = function(view) ... end,  -- true when it fires
}
```

Rules are sorted by priority, highest first, and the first that matches answers. A rule may
answer `nil` as its state (herdr's `skip_state_update`) to mean "this screen is something else
entirely — a transcript viewer, a model picker — do not read it as any state at all", in which
case the previous answer stands.

`visible` records *why* it fired: `idle` and `blocked` mean the screen is showing an idle
prompt or a question right now, as against an absence of evidence. Only the hold reads it.

### 3.3 The hold

A working agent's spinner disappears for a frame between steps. Without a hold, that frame is
an `idle`, which is a sound in the other room and a sidebar that blinks.

So: a fall from `working` to `idle` is refused for `settle` milliseconds — unless the new
answer is *visibly* idle or blocked, in which case it is believed at once. Every other
transition is immediate. herdr counts confirmations as well as time; this counts time only,
because neotmux looks at a pane on every frame it changes rather than on a timer of its own,
so the confirmations would be a count of frames, which is a count of how chatty the pane is.

### 3.4 The Claude ruleset

Ported from `claude.toml`, keeping its priorities so the order is comparable:

| priority | id | state | what it reads |
|---|---|---|---|
| 1100 | `osc_title_working` | working | the title starts with a braille char or a half-circle |
| 1000 | `transcript_viewer` | — | "showing detailed transcript" and a hint line: not a state |
| 980 | `live_blocked_form` | blocked | after the last rule: "esc to cancel" with "enter to confirm"/"enter to select" |
| 975 | `btw_overlay_working` | working | the last five lines hold `/btw` or "esc to close" |
| 970 | `live_turn_working` | working | `⏸`/`⏵` with "esc to interrupt", or a spinner glyph with `…` |
| 950 | `live_prompt_box` | idle | the prompt box holds `❯` and no menu hints |
| 900 | `model_picker_menu` | — | "select model" + "enter to set as default": not a state |
| 850 | `bash_permission_prompt` | blocked | "do you want to proceed?" plus a Bash tell and a yes/no option line |
| 840 | `generic_permission_prompt` | blocked | the same, after the last rule, without the Bash tell |
| 300 | `legacy_no_prompt_blocker` | blocked | the older shapes: "do you want to", "waiting for permission", … |
| 250 | `osc_title_idle` | idle | the title starts with `✳` |

Rules that read the title are worth more than rules that read the screen, which is why they
bracket the table. A pane whose title Claude never set falls through to the screen rules, which
is every pane running something else.

Two of herdr's rules are not ported: `background_agents_working` and
`background_mcp_task_working` read shapes this project cannot test against a real screen, and a
rule nobody can check is a rule nobody can fix. The table is a table; they can be added.

## 4. `neotmux.state` keeps its face

`state.of`, `state.worst`, `state.icon`, `state.icons` and `state.forget` all keep their
signatures. What changes is underneath: `of` asks `neotmux.detect` first, and falls back to the
existing pattern list for a pane no ruleset claims — which is what a pane running `psql` or
`ssh` is. The eight patterns and `M.lines` stay exactly as they are, and so does the
revision-keyed cache that keeps a quiet pane free.

`M.working_for`, the two-second window, becomes milliseconds against `neotmux.now()`. It still
only applies to the fallback: a ruleset that says `idle` is believed.

## 5. `plugin.chime`

The sound, as a shipped plugin, on by default.

```lua
local chime = require('plugin.chime')
chime.sounds = {
  blocked = 'sounds/request.mp3',   -- relative to the layer the plugin came from
  idle = 'sounds/done.mp3',
}
chime.only_when_away = true         -- default: silent for the pane you are looking at
chime.players = { ... }             -- see below
```

On every tick it reads `state.of` for each pane of the session and, when a pane's state
*changes into* one that has a sound, plays it with `neotmux.job.start`. A pane that was already
blocked when you looked away does not chime again; a pane that goes `working → blocked` does.
`only_when_away` is the difference between a useful sound and an annoying one — the pane you
are watching does not need to make a noise at you.

### 5.1 Finding a player

The sounds are mp3, and mp3 is where audio players stop agreeing. `paplay` refuses it,
`aplay` plays it as raw PCM — noise at full volume, which is worse than silence.

So a player is not judged by whether it *started*: it is judged by how it *exited*.

```lua
chime.players = {
  { 'paplay' },
  { 'pw-play' },
  { 'ffplay', '-nodisp', '-autoexit', '-loglevel', 'quiet' },
  { 'mpg123', '-q' },
  { 'mpv', '--no-video', '--really-quiet' },
}
```

The list is herdr's, in its order, and for its reason: every one of them decodes mp3 or refuses
cleanly, and `aplay` is deliberately absent. The first chime tries them in order — `start`
answering `(nil, message)` is "not on this machine", and `on_exit` with a non-zero code is
"cannot play this" — and the first that exits zero is remembered for every chime after. If none
work, the plugin says so once with `neotmux.notify` and stops trying.

A chime's job is a job like any other: it belongs to its source and its session and goes when
either does. A player that hangs is not a problem the plugin has to solve — it outlives nothing.

## 6. What is tested, and how

The detector is a pure function of a view, which is the whole reason for building it that way:
the tests are screens in, states out, with no pty and no timing.

- **A fixture per rule**, taken from a real Claude Code screen and trimmed: a working spinner, a
  Bash permission prompt, a plan-mode form, an idle prompt box, a transcript viewer, a model
  picker. Each asserts the state *and* which rule id fired, so a fixture that starts matching a
  different rule is a failure rather than a silent pass.
- **The title beats the screen**: a screen that reads as idle under a title that says working is
  working.
- **The hold**: working, then a screen with no evidence either way, answers working until
  `settle` has passed and idle after; the same fall with a *visible* idle prompt answers idle at
  once.
- **The fallback**: a pane whose screen matches no rule and whose title is empty gets the old
  pattern list, and a `[y/N]` still reads as blocked.
- **`plugin.chime`**: a fake job runner, the harness `tests/jobs.rs` already has, plus a scripted
  sequence of states — one sound per entry into a state, nothing for the focused pane, and the
  walk down `players`: one that will not start is skipped, one that starts and exits non-zero is
  skipped, the first that exits zero is remembered and the ones before it are not tried again.

The host's two additions get their own: a pane that sets a title reports it and a pane that
never did reports nil; `neotmux.now()` moves forwards and never backwards across a frame.

## 7. What this does not answer

The detector says what a pane is doing, not what it said. A sidebar that shows the last thing
an agent asked, how long it has been waiting, or which pane to jump to next, wants more than a
state — and wants `neotmux.list` and a tree beside it (#35). That is the next thing, and this
is what it would stand on.

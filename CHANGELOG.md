# Changelog

## Unreleased

### A long branch ends in an ellipsis

A session name or git branch too long for the sidebar was cut at the band's edge, taking the
branch's `↑`/`↓` counts with it. Both now end in `…` where they would overflow, and the counts
stay. An agent row's session, window, agent and command, and a window name in the tree, do the same.

### One place for every plugin's colours

Every shipped plugin now reads its colours from `neotmux.style`, and every colour is a name in
`neotmux.theme` — the 26 Catppuccin Mocha colours, which is what they all were. Restyle every
float, one plugin, or one part of it with `require('neotmux.style').set('float', { frame =
'crust' })`; recolour everything at once with `require('neotmux.theme').set({ blue =
'#7aa2f7' })`. **`require('plugin.x').colors` is gone**: `require('plugin.tmux.status').colors.
accent = '#ff0000'` is now `require('neotmux.style').set('status', { accent = '#ff0000' })`, and
`docs/plugins.md` lists every selector and its parts. Thirteen themes ship in `neotmux.themes` —
the four Catppuccin flavours, tokyonight, gruvbox, nord, dracula, rose-pine, kanagawa, onedark
and everforest — switched with `require('neotmux.theme').use('<name>')`, and a new `theme`
command in the palette picks one from a list; nothing is written to disk, so keeping a pick
means adding that line to `init.lua`.

### Resize moves the boundary the way the key points

`h` and `l` in resize mode grew the pane at whichever end was named, so the middle of three
columns widened on both sides and never narrowed. `neotmux.pane.resize` now moves the pane's
right (or bottom) boundary in the direction given, and its left (or top) one only when the pane
is against the far edge — tmux's rule. `left` then `right` puts the line back where it was.

### The log cannot fill the disk

A plugin that raised on every tick wrote its error, stack traceback and all, four times a
second for as long as it ran — one day of that came to 200 MB. A warning or an error that only
repeats the last one from the same source is now counted instead, and written again with the
count when the source says something else or a minute has gone by. Past 10 MB the log is moved
to `neotmux.log.1` and started afresh.

### `costs` lines up, and draws each source's last ticks

The source column was 24 wide, so a name like `plugin/herdr/sidebar/spaces` pushed its row out
of line. It is now as wide as the longest name, the numbers sit under a heading instead of
between labels, and the rest of the row is a sparkline of that source's last ticks, all drawn
against one scale so a bar means the same time in every row. `neotmux.costs()` rows carry
`recent_ms`, the last 120 ticks oldest first, and `neotmux.list` takes a `header`.

### A busy machine no longer parks the plugins

The tick budget measured the wall clock, so a parallel build that took the CPU away from
neotmux in the middle of a tick counted against the plugin, and five of those in a row parked
it: the sidebar, the chime and `neotmux.seen` went quiet whenever cargo ran. The 8 ms is now
the thread's CPU time. A tick is only over once it has run past the deadline on the wall clock
and used more than its budget of CPU.

### A plugin set up from your `init.lua` keeps its prefix key

Setting a shipped plugin up from your own `init.lua` — `require('plugin.stamps').stall = ...`,
as its header suggests — loaded it before the shipped layer had loaded the prefix, and `C-b t`
was silently left unbound; `C-b e`, `C-b C` and the session keys had the same trouble. Every
shipped plugin now binds its prefix key through `neotmux.bind`, which loads the prefix when the
defaults say it is about to be loaded anyway.

### `C-b e` stays cheap however far copy mode winds back

The exits band read every mark from the top of the view to the end of the pane on every tick,
and with a few thousand commands in the history copy mode wound back far enough to stop it.
It now reads only the rows it draws, and the marks just past them that settle the last command.
`neotmux.pane.marks` takes a fourth argument, the most marks to return, and
`neotmux.commands.of(id, first, last)` reads a range of lines.

### The times and the exits follow copy mode

`C-b t`'s times and `C-b e`'s marks stayed where the live screen's lines were while copy mode
scrolled the pane back, so each sat beside the wrong line. They now wind back with it.
`neotmux.pane.view(id)` is what they read: how many lines back `ctx:view` has a pane drawn this
frame.

The pane bands' cap goes from 16 columns to 24, so `C-b t` and `C-b e` can be shown together.

### Shrinking the terminal keeps the bottom of every pane

A shorter terminal used to cut rows off the bottom of every pane's screen, so the output you had
just read was gone, and growing the terminal back did not bring it back. Now the rows above the
cursor scroll into the history first, as they do in any terminal: the line being written stays
on screen, and everything above it is still there to scroll back to. The times `C-b t` shows and
a shell's marks stay on the lines they belong to.

### How each command ended, beside it

`C-b e` (the palette's `exits`) shows a band down each pane's left edge with, on the line every
command was typed on, `✔` for a zero exit, `✘` and the status otherwise, or `⋯` while it runs,
and how long it took. It is `plugin.exits`, it reads `neotmux.commands`, and it needs a shell
that sends OSC 133 marks. `require('neotmux.defaults').exits = false` keeps it out.

### `C-b x` ends a pane

`C-b x` now ends the focused pane rather than the window, as tmux's does; the window moves to
`C-b &`. `neotmux.pane.kill(id)` is what it calls.

### A pane's commands, from its shell's marks

A shell that sends OSC 133 marks says where its prompts, command lines and outputs start, and
how each command exited. neotmux now keeps those marks against the lines they landed on:
`neotmux.pane.marks(id)` reads them, and `require('neotmux.commands').of(id)` turns them into
commands, each `done`, `running`, `prompt` or `unknown`. A shell that sends none behaves exactly
as before. Nothing is drawn yet. `docs/plugins.md` has the lines to add to bash or zsh.

### `neotmux.pane.outer`

`neotmux.pane.outer(id)` answers a pane's rect with its pane bands, the way `ctx:pane_outer`
does inside a tick. The pickers and the palette are now placed over the panes' outer rects, so
`C-b t` no longer moves them across or takes their columns, and `neotmux.nav` compares outer
rects, so a pane too narrow for its band is still found above or below its neighbours.

### The times have a band of their own

`C-b t` no longer paints over the first nine columns of every pane. While the times are shown,
every pane's shell is ten columns narrower and the times sit in the columns it gave up, their
text yellow or red after a wait rather than the cell behind them. Hiding them gives the columns
back. `require('plugin.stamps').colors.gutter` is gone and `rule` is new.

The band is `neotmux.reserve_pane('left', cells)`, which any plugin can hold: `ctx:pane_band(pane)`
answers where a source's columns of a pane are, and `ctx:pane_outer(pane)` the pane with its
bands, which the borders and the prompt now draw around. `ctx:pane_rect` and
`neotmux.pane.rect` are what they were — the part of the pane the shell writes in.

### Closing a window or a session from the prefix

`C-b x` ends the window you are in and every shell in it, and ending the last one ends the
session; `neotmux.window.kill(id)` is what it calls. `C-b N` makes a session and goes there,
without the picker, and `C-b X` ends the session you are in, moving you to another first when
there is one. Clearing the notifications moves from `C-b N` to `C-b C`.

### A sidebar runs the full height

A band on the left or right now takes the terminal's full height, and one on the top or bottom
only the width between them, as herdr lays its sidebar and tab bar out: the status bar and the
footer start beside the sidebar rather than over it. `ctx:reserved()` answers the new rects, and
the panes' area is the same as before.

### The status bar is herdr's tab bar

The top row is now herdr's tab bar: a tab for each window, the name you gave it or its
position, ` Z` after it while zoomed, the window being shown in blue, and a strip that scrolls
to keep it in view when there are too many. The focused pane's command and directory are gone
from the right; `require('plugin.tmux.status').right` is a list of functions whose text goes
there instead, and is empty to begin with, as herdr's is. `caps`, `icons` and `home` are gone
with the old look, and the colour fields are renamed after what they colour now.

### What tmux does is `plugin.tmux`

The six shipped plugins that do what tmux does — the status bar, the prefix, copy mode, the
session picker, the borders and synchronized panes — moved to `plugin/tmux/` and are loaded
together by `plugin.tmux`. `require('plugin.prefix')` is `require('plugin.tmux.prefix')` now, and
the same for the other five; their sources are named `plugin/tmux/...` in logs and in `costs`.
Each is still turned off by its own field in `neotmux.defaults`, and `defaults.tmux = false`
turns off all six.

The three that follow agents the way herdr does — the sidebar, the chime and the watch rules —
moved to `plugin/herdr/` the same way: `require('plugin.herdr.sidebar')`, sources named
`plugin/herdr/...`, and `defaults.herdr = false` turns off all three.

### When every line arrived

`C-b t`, or the palette's `timestamps`, shows down each pane's left edge the time each line
arrived, where it differs from the line above, on a gutter coloured by how long the pane sat
before it: yellow after two seconds, red after ten. Every pane's times are kept from the start,
hidden windows included, and a line takes its time with it when it falls off the history. A
progress bar rewriting its row keeps the time it first appeared, and a full-screen program's
rows get none.

### A plugin can read what colour a cell is

`neotmux.pane.content(id, { runs = true })` answers each row as the stretches it is drawn in,
each with its text, its foreground and background — a palette index, `#rrggbb`, or nil for
the terminal's own, the way the draw verbs take them — and whether it is bold, dim, italic,
underlined or inverse. Output a program coloured itself can be told from plain text now.

### An idle session costs a thirteenth of what it did

The plugins tick only when something happened to the session — a key a plugin took, output
in a pane, a resize, an event, a job's output, a timer coming due, a notification appearing or
going — and otherwise four times a second, instead of sixty. A frame with nothing happening
shows what they drew last. Measured with the sidebar on and nothing happening for about twenty
seconds: 109 ticks where there were 1380, and the sidebar's 803 ms down to 64. A thing that
changes with time alone, like a notification timing out, is redrawn up to a quarter of a second
late; a plugin that needs it sooner arms a timer.

### What each plugin's ticks cost

The palette's `costs` lists every source with how many ticks it has run since the config
loaded, their time together, the mean and the longest, most expensive first, counting up while
it is open. The host times each source's whole tick; `neotmux.costs()` reads the same numbers.

### Every text field takes the same keys

A prompt — naming a window, a new session from the palette — a list's query and the question a
list asks all edit with the same keys now: Backspace and `C-h` take a character back, `C-w` the
last word, `C-u` the whole line. The prompt used to take only Backspace. Taking a character back
off Japanese left a byte of it behind in all three, and takes the whole character now. The keys
are `neotmux.line`, for a plugin with a field of its own.

### A yank reaches the session it was made in

With a client on each of two sessions, a yank in copy mode went to the terminal of whichever
session's frame came first, and the client that yanked got nothing. A yank is kept with the
session it was made in now, and only that session's clients are sent it. A yank in a session
nobody is looking at says so rather than claiming a write.

### No flicker under a build

A frame is written as one synchronized update (DEC mode 2026) with the cursor hidden while its
cells are written. Under a pane pouring out a build, a terminal used to paint a frame as it
arrived, half over the last one, with the cursor racing across every cell being written. A
terminal that does not know the mode ignores it, and still gets the cursor hidden.

### `C-b ?` lists the keys

`C-b ?` opens every prefix key in a list, as tmux's `list-keys` does: the key, the command it
runs and what that command does. Enter runs the one under the cursor. A key a plugin bound to
a function says what it does through `require('plugin.prefix').descriptions`.

### Sessions are numbered, not `default`

`neotmux` with no name starts a new session every time, as tmux does, called the lowest number
no running session is: `0`, then `1`, then `2`. `neotmux rename 0 work` gives one a word when it
wants one. Coming back to a session is `neotmux 0`, and `neotmux kill` now needs the name. A
command given no `-t` goes to the only session there is, and asks which when there are more.
`neotmux new` and the session picker's `c` with no name are numbered the same way. This is
protocol 16: an attach may name no session.

### `C-b a` walks the agents

`C-b a` puts a cursor on the sidebar's agents, as `C-b w` does on its sessions: `j` and `k`
move it, Enter goes to the agent under it — its session, its window and its pane — `1`-`9` go
straight to one, and Esc leaves. Only one of the two cursors is up at a time.

### The sidebar's rows are herdr's

Every session in the sidebar is two rows, as in herdr: its name, then its branch, or nothing
where there is no branch, so the sessions below do not move when one leaves git. A band too
short to show the pane you are in gives up those empty rows first. An agent's second row says
what state it is in before what it is: `working claude`.

### Zoom

`C-b z` gives the focused pane the whole window, as tmux's does, and pressing it again puts
the window back. The other panes are hidden rather than closed: their shells run and their
terminals keep their size. The bar writes `Z` after the pane's number while it is zoomed, and
moving to another pane, splitting or resizing unzooms first. For a plugin, it is
`neotmux.pane.zoom()` and `neotmux.pane.zoomed()`.

### Twenty-one more agents, by herdr's own rules

A pane running Codex, Gemini, Cursor, OpenCode, Copilot, Amp, Kiro, Qwen or any other agent herdr
has a manifest for is read the way herdr reads it: `working`, `blocked` and `idle` come from
herdr's rules for that program, run by `neotmux.manifest`, a port of herdr's engine. Which agent
a pane is running is herdr's reading of its foreground job, so an agent started through `node`
or a wrapper is found too: `neotmux.pane.info(id).job` is every process of the pane's foreground
job with its argv, and `require('neotmux.agent').of(info)` says which agent that is.
The rules are written in Rust's regex syntax, which a plugin can use now too, as
`neotmux.regex.is_match(pattern, text)`.

### Two plugins can hold the same pane still

`neotmux.pane.freeze(id)` hands back a hold, and a pane is frozen while anyone holds it:
`hold:release()` lets go of that one hold and nobody else's, so one plugin can no longer thaw a
pane out from under another. `g.pane.freeze(id)` is the same through a group, and the group
releases it when it is emptied, so a reload never leaves a pane frozen by a hold taken that
way. `neotmux.pane.unfreeze` is gone. Copy mode and hints take their holds this way and no
longer need to ask `pane.frozen` first.

### Faint text is drawn faint

Text a program draws faint (`ESC[2m`, "dim") comes out faint in a pane, instead of at full
intensity: the parser always knew, and now the cell carries it across the wire to the screen.
That is a new cell flag, and protocol 15.

### A pane's shell is not a login shell, and knows the terminal it is in

A pane runs your shell as an interactive shell, not a login one, so `.bash_profile` and its kin
are not run again for every pane. Its `TERM` is `xterm-256color`, the emulator it is actually
writing to, rather than whatever terminal the server happened to be started from — or nothing,
for a server started from a script.

### The config reloads without losing the panes

`config-reload`, from the palette or anything that runs a command, runs every layer's
`init.lua` again in the running server: the panes, the shells and their scrollback stay, and the
plugins are new. `neotmux.reload()` is the function under it, for a key of your own. It is safe
because a plugin registers through `neotmux.group(name)`, which is Neovim's augroup: every
`on_tick`, key, band, float, job and timer made through the group is taken back when the group is
named again, and a group the reload did not name is emptied, so a deleted `require` line is a
plugin gone. The `unload` event fires once before, for what no group can see. A source setting
a key or adding a command it already holds now replaces its own instead of being refused, and
`neotmux.command.remove(name)` takes a command away. Every shipped plugin and example registers
through a group. A reload says whether it went well on the notification surface: each source that
failed to load, at warn, or `config reloaded` when nothing did.

### The sidebar is herdr's

The shipped sidebar is herdr's, so moving over is nothing to relearn. On top, `spaces`: every
session with its state and name, then the git branch of the directory its focused pane is in and
how far that is ahead of (`↑`) and behind (`↓`) its upstream. Under it, `agents` in herdr's rows
— where on the first, which agent on the second — in session order, or by urgency with
`agents.sort = 'priority'`. herdr's colours and icons, and its **done**: an agent that went idle
where nobody was looking stays done until somebody looks, which needed
`neotmux.session.attached(name)`, how many clients are at a session, since every session ticks.
`C-b w` is herdr's navigate, a cursor over the spaces with Enter to go and `1`-`9` to jump, and
`C-b g` herdr's goto: the palette on its agents, Enter to focus one in whatever session it is.

And it is drawn as herdr draws it, read off a real herdr 0.9.1 row by row: 26 columns on the
terminal's own background with only the rows you are at painted, bold headers over a blank row,
the band cut at half with a line across, a session's state taken from its agents alone (`·` with
none), herdr's colours and weights for names and branches, `«` in the corner, a four-column rail
of numbered states when folded, and `NAVIGATE` along the bottom while the cursor is up. A draw's
text can be bold for it — `ctx:text(..., { bold = true })` — which is protocol 13.

The palette takes providers: `@` lists the sessions, `#` the agents, anything else the commands,
and a plugin adds its own to `require('plugin.palette').providers`. `agent-next` and
`agent-prev` step through the agents, bound to nothing. The tree the sidebar was is still shipped
as `plugin.sidebar.sessions`, one assignment away, and requiring `plugin.sidebar` from your own
`init.lua` no longer loses `C-b b`.

### A session can be made and renamed from the palette

`session-new` asks for a name, makes the session and goes there; `session-rename` asks for a new
name for the one you are in. They are the session picker's `c` and `r`, registered as commands.

### The space you are in shows its windows and panes

Under the session you are in, `spaces` draws its windows and panes as the tree does, so where
you are inside it is on screen as well as which session it is. `spaces.tree = false` leaves them
out; when they do not fit, every space keeps its name first.

### What you type straight after a switch goes to the session you switched to

A key a plugin took and acted on — `session.switch`, `pane.focus`, `pane.send` — is carried out
by the next frame, and the keys that came in the same read after it went to the old session's
pane before it was. Typed quickly after a switch, the first letters landed in the session you had
left. A client's input now waits after such a key until the frame that carries it out, which
is also what a test that failed one run in many was catching (#44).

### Killing a job can no longer signal somebody else's processes

A job's waiting thread reaped the child and only then cleared the pid a kill would signal, so a
kill landing between the two sent SIGKILL to the process group of whatever the system had handed
that pid to next — the job's whole group, since jobs run in their own. It now waits for the exit
with `waitid(WNOWAIT)`, which leaves the child a zombie still holding its pid, clears the pid
while it is, and reaps after.

### The first list opened no longer flashes in the corner

The first `neotmux.list` a session opened — the palette, the session picker — was created at the
pane area's top left and moved to its place a frame later, because the only thing that knew the
pane area was the list's own tick, and that tick is armed by the first open. `open` now reads the
area itself, through `neotmux.pane.rect`, before it creates the float.

### The Lua is checked too

The shipped runtime, the examples and the test plugins are formatted by `stylua` (`.stylua.toml`)
and linted by `luacheck` (`.luacheckrc`), in `make check`, in a CI job of their own, and on commit
through `.pre-commit-config.yaml` alongside `cargo fmt`. Bringing the code up to it changed no
behaviour: the formatting, one comment and one message string rewrapped, one variable renamed
where two meant different things (`list.lua`'s list top), one unused argument marked unused.

### The crates say what they are

The crates sit at the repository root: `neotmux-proto`, `neotmux-layout` (was `neotmux-core`),
`neotmux-pty` (was `neotmux-term`), `neotmux-lua`, and the old `neotmux` split into
`neotmux-client`, `neotmux-server`, `neotmux-sys` and the binary. The client depends on proto and
sys alone, so it cannot reach Lua, a PTY or the layout. Every `lib.rs` and `mod.rs` is a list of
exports, and files are grouped into directories by what they are for. Nothing behaves
differently.

### The sidebar is a stack of sections, and the second one lists your agents

`plugin.sidebar` is a container now: `require('plugin.sidebar').sections` is a list of tables,
each with a `build` that says what rows it has and an optional `fit` that cuts them to a height,
and a user adds, drops or reorders them from `init.lua`. The session tree is the first; its
layout fields (`sessions`, `windows`, `panes`, `tokens`, `tree`, `markers`) moved with it to
`plugin.sidebar.sessions`, with no aliases left behind. The tree also stops losing sessions off
the top when the one you are in has many panes: it gives up those panes first, saying `… n more`
where it cut them. The second section, `plugin.sidebar.agents`, lists every pane an agent
ruleset answers for, in every session, blocked first, with how long it has been in its state —
which needed `neotmux.detect.agent(pane)`, the name of the ruleset that answered.

### A pane that starts waiting for you says so out loud

`neotmux.state` used to guess with eight patterns over a pane's last five lines — enough to
tint a row in a sidebar, and not enough to wake you up. The guess is now a detector:
`neotmux.detect` reads a pane's screen as regions (the tail, what is under the last rule, the
prompt between the last two) and judges it with a ruleset of prioritised rules, one file per
program it has been taught. The shipped one is `neotmux/agents/claude.lua`, eleven rules for
Claude Code, six of them checked against screens captured off a real one. A fall from working
to idle is held for a moment unless the screen says idle in so many words, because the gap
between two frames of a spinner is a screen that means "busy" and a notification fired on that
blink is why this kind of detector gets turned off. `state.of` asks the detector first and
keeps the pattern list for a pane no ruleset claims and for one whose rules all had nothing to
say — a shell at a prompt, a pager, a build. A ruleset says which panes are its through
`claims(info)`; the shipped one names the shells and speaks for everything else, because the
command a pane reports for an agent is whatever you launched it through, and `claude` under
`systemd-inhibit` or `npx` is still Claude Code.

What the detector is for is `plugin.chime`, on by default: a sound when a pane you are not
looking at starts waiting for you, and another when it stops working. One sound per entry into
a state, so a prompt that is still up next frame is the same wait. It finds a player by trying
`paplay`, `pw-play`, `ffplay`, `mpg123` and `mpv` in turn and judging each by how it *ends*
rather than by whether it started — one that is not installed fails at the spawn, one that
cannot decode an mp3 starts happily and exits non-zero — and remembers the first that exits
zero. When nothing can play a file, what it says names the file and the last failure rather
than blaming the list — the fault is as easily a missing path or a machine with no audio — and
it says it once per file, so one bad sound does not silence the other. Assigning `players` or
`sounds` is read as "try again", because somebody who has just fixed their config should not
have to restart the server to be heard. The two sounds are carried from herdr, as are
the rules and the hold; `NOTICE` at the root says which files and from where.

### The wire lost the question nobody was asking any more

When the Lua layer lived in the client, a plugin reading a pane's text had to ask the
server for it across the socket and wait for the answer to come back. The layer runs in
the server now, beside the panes it reads, so that question has been a function call for
several versions while the messages that used to carry it sat on the wire unsent. They are
gone: the read, the answer, the client-side asker that spoke them, and the server's arm
that replied. Nothing a plugin can do changes; `neotmux.pane.content` and its neighbours
read the same terminals they were already reading.

`PROTOCOL_VERSION` is now 12, because the wire's vocabulary is smaller than it was, so a
client and a server built either side of this refuse each other by name and number rather
than misreading a byte.

### A plugin can run a program, and ask to be called again later

`neotmux.job.start` runs a program in the server and calls you back with what it said;
`neotmux.timer.every` and `neotmux.timer.after` call you back when the time comes. Both
are what the plugin layer was missing to do the thing it exists for — watching something
long-running while nobody is looking at the pane it would have needed — without a pane, a
coroutine to keep alive, or a shell to hide it in.

A job takes its command as a list, so nothing is split on spaces or globbed behind your
back, and a table of options beside it: `cwd`, an `env` that is added to the server's
rather than replacing it, and the callbacks. Buffered — the default — hands `on_exit` the
whole of what the program said; streaming hands `on_stdout` and `on_stderr` complete lines
as they arrive, holding back the half-line a chunk boundary cut and flushing the last
fragment before the ending. `on_exit` runs exactly once, whatever ended the job, and says
which of the ways it was. `start` answers the handle, or `nil` and one line saying why:
there is no error to catch and no coroutine to drive.

A timer is clamped to the frame, fires at most once per frame and does not catch up — five
callbacks to make up for a slow frame is a worse answer than one. `every` and `after`
answer the same two values `start` does, and have the one reason to refuse: an arm from
inside the `on_exit` a session teardown hands you, whose timer would belong to a session
with no frame left to fire it. Both run before the tick
in the frame they arrived in, so a tick draws what they changed rather than finding out
next frame; both are measured against the same tick budget everything else is, where a
runaway callback costs its own job or its own timer and nothing else its author wrote; and
both belong to their source and their session, so a plugin that goes takes its jobs with
it, a session that ends kills its jobs and tells them so, and a job is killed by its whole
process group rather than only by the program's own pid — a server that is asked to stop
takes them with it, and only a `SIGKILL`ed one, which runs nothing on its way out, leaves
them behind. `neotmux.opt.job_concurrency` and
`neotmux.opt.job_output_bytes` bound what all this may cost.

### Two clients in one session

`neotmux work`, in two terminals at once. A key typed on one is the key the other
sees, the session's and the pane's rather than the terminal's, and a plugin on one
terminal can be in a mode the other is not, the same way one can be in a mode and
the shell not be. The one that goes costs the other nothing, and the session the two
of them are in goes on until its last shell exits or any one of them calls
`neotmux.quit()`, which is to say, as long as a plugin on either of the two is still
asking for what it asked for — the OR, and not the last word, which is what it used
to be, and what "the last tock wins" would have made it.

### Sessions by name, and `kill` where `--kill` was

`neotmux work` starts a session called `work` where nothing is running and attaches to
it where one is; `ls` shows the running ones, one per line with the server's pid, and
`kill [name]` ends one of them, `default` where no name is given — which is what a bare
`kill` does, and what `--kill` used to be, and is no more. A name is letters, digits,
`-` and `_` (none first), one to sixty-four characters, because it becomes a filename,
and a name that would not is refused before anything is created. `C-b d` detaches to the
same server it always did; running `neotmux work` again finds the session, as running
`neotmux` again already found `default`. Nothing about the wire changed: the name is a
path, and the socket was one file per session the day it was written.

### The Escape key, in every shape a terminal can send it

A terminal that uses the `CSI u` keyboard encoding — kitty's protocol, which Ghostty and a
few others speak — sends a plain Escape as `CSI 27 u` and a plain `a` as `CSI 97 u`. The
input decoder recognised no `u`, and its rule for well-formed sequences with no key is to
consume them whole, so those keys were silently dropped: a `ctl-[` that never reaches the
program. Now the `u` form is read back — the first parameter is the key's Unicode codepoint,
the second a modifier mask — and the key is delivered exactly like any other. Every other
form is untouched, and a codepoint with no key is still swallowed rather than typed.

### A pane begins where it is asked to begin

A new pane's shell used to start in the user's home directory, wherever neotmux was run from
and whatever the pane it split from was in: the working directory was never named, and the
library neotmux forks through turns an unset one into `$HOME` rather than leaving it
inherited. Now the first pane begins in the directory neotmux was started from — the
multiplexer behaves like every other terminal a hand can drop it into — and a split, and a
new window, begin in the focused pane's own directory, the read the status bar already uses
for `#{pane_current_path}`-style answers, from the process the pane is running. When that
directory has been deleted out from under the pane, the answer is the startup directory
rather than the home one, and the split still happens.

The decision, in one place: the shell neotmux starts is still a login shell (the `-` argv0
the spawner's default uses), and there is still no way for a plugin opening a pane to choose
the directory — both known, and neither changed by this.

### A session outlives the client, on purpose and by accident

The three-second linger that used to end a session shortly after its client left is gone. A
session now ends only when its last shell exits, a plugin calls `neotmux.quit()`, or the
server is signalled — whether the client left on purpose or its terminal just went away.

`C-b d` (or `neotmux.detach()` from a plugin) is the purposeful case: the client leaves and
exits 0, and the server keeps the session and goes back to waiting. Running `neotmux` again
finds that session and is shown it — `Session::adopt` refits it to the new terminal, the new
client's plugins declare their own bands and gutter again, and the geometry it publishes is
how those plugins learn what is already there. A window id counter now takes its watermark
from that published geometry too, so a re-attached client cannot mint an id that already
exists.

`neotmux --kill` ends a session from outside the protocol, for the one case that needs it: a
rebuilt client meeting a server that refuses its handshake, with no message either side will
accept. It goes by the pid the server writes into the lock file it holds while running, and it
refuses to signal unless that lock is actually held and `/proc/<pid>/comm` says the pid really
is a neotmux — a stale or recycled pid is not enough. With nothing running it says so and
exits non-zero. `neotmux --help` and `neotmux --version` are new alongside it; `--version`
prints the protocol number beside the package version, since that is the number a person needs
after reading a refusal.

Two things worth knowing. `PROTOCOL_VERSION` is now 3, so a client and a server built from
different commits refuse each other by name and number, the same as before. And a re-attach is
a plugin restart, not a resume: the panes, the shells, the layout and each pane's scrollback
are the server's and come back as they were, but a plugin's own state — a copy register, a
raised mode, a folded sidebar, and every reservation — is the client's, dies with it, and is
re-made from scratch when the plugin loads again.

### Two processes, one socket

Neotmux is now a server that holds the session and a client that holds the terminal, talking
over a unix socket at `$XDG_RUNTIME_DIR/neotmux/default.sock` — `/tmp/neotmux-<uid>/` where
that variable is unset, and one file per session so that a second session is a second name in
the same directory rather than a protocol. Running `neotmux` still just starts neotmux: with
nothing listening it starts the server itself and then attaches to it.

The server owns the panes, their children, the layout and the terminal state behind each
pane. The client owns the terminal, raw mode, key decoding, the drawing — **and the Lua
layer**, which is where M3 puts it: the server holds other people's long-running jobs, so a
plugin that freezes should cost one client rather than every session. A frame is one exchange:
the server says what happened and what the session looks like, the client ticks its plugins
against it and answers with the ops they raised and what they drew, and the server applies
that and sends back the picture. Keys that no plugin bound, resizes, and the pane reads
`neotmux.pane.content` makes all cross the same socket.

`neotmux.clipboard.set` writes OSC 52 to standard output, and standard output is now the
client's terminal in the process that runs the plugin — the same terminal ratatui draws on,
which is what the call always claimed.

Two things about this split are still worth knowing: while a session has a client, a second one
is turned away with a reason rather than left waiting; and a client and a server built from
different commits refuse each other by name and number rather than trying to survive it, so
replacing the binary under a running session means restarting it. "Nothing about using neotmux
changed" no longer holds either: `C-b d`, `neotmux --kill`, `--help` and `--version` are also
new.

### Pasting more than 1024 bytes

A paste larger than 1024 bytes used to lose its tail. Exactly one bufferful arrived; the rest
sat in the terminal until the next keystroke shook another bufferful loose, and while it sat
there neotmux held a core at 100%.

Neotmux now reads its own terminal and parses its own keys. The read drains until the terminal
has nothing left, so a paste of any size arrives in one pass, and the keys come out of a parser
that covers what the program binds: characters including UTF-8, the control characters, the
CSI and SS3 forms of the arrows, the function keys, Home/End/PageUp/PageDown/Insert/Delete,
their xterm modifier parameters, and Escape.

Two consequences worth knowing. An `ESC` with nothing behind it in a fully drained read is the
Escape key, decided at once with no timeout to wait out; an `ESC` with more bytes behind it
begins a sequence. And a sequence neotmux has no key for — a mouse report, a focus
notification — is consumed and discarded rather than typed into the pane, since neotmux turns
none of those modes on and the pane's child never asked for them.

Nothing about output changed: ratatui and crossterm still draw the screen.

### Copy mode

`C-b [` enters a copy mode over the pane you are in: `h/j/k/l` and `C-u`/`C-d` move a cursor,
`g` and `G` go to the ends of the buffer, `Space` starts a selection, `y` yanks it and `Esc`
or `q` leaves. It ships on, with the status bar and the prefix rather than with the watcher
and the sidebar: it takes no band, no gutter and no cell from any pane, and what it reads is a
buffer the host is already keeping whether or not anything looks at it. One line turns it off:

```lua
require('neotmux.defaults').copy = false
```

It is a plugin, and the host learned nothing. The mode is a local and an `on_key`, the way the
prefix's own two modes are; the cursor and the selection are `ctx:tint`, so the pane's real
text stays readable under them; the keys are a table of command names over a table of
commands, so rebinding one is an assignment from an `init.lua`.

**The buffer is taken once, on the way in.** The pane's whole history and its screen are read
in a single pass and never read again, because the child keeps printing while you read: a
buffer re-read every frame is one whose line 400 is one thing when the cursor lands on it and
another when it is yanked. A ten-thousand-line history costs about 11 ms to snapshot, on a key
press and outside any tick. The frames after it are free of reads entirely — sitting on the
pane's own screen a frame is one tint, and scrolled it is one `ctx:rect` and one `ctx:text` per
row, 8–32 µs against a budget of 8 ms.

**The selection is two absolute line numbers**, never screen rows, so scrolling past one of its
ends clamps it rather than losing it.

Yanked text goes into `require('plugin.copy').register` and not into the system clipboard,
because this ABI cannot reach one. OSC 52 would need a plugin to write bytes to the outer
terminal, and there is no such call — `io` is nil in the VM and every drawing call puts a
character into a cell of the grid, where an escape byte is a glyph. Writing the text into a
pane would need a way to send input to one, and the pane API splits, focuses and resizes.
Neither gap was closed here: adding a host call was not what copy mode was for.

Two defects are stated rather than guessed at, both for want of something the ABI does not
say. A soft-wrapped line is yanked with a newline in the middle of it, because no row carries
a wrap flag and a wrapped row reads identically to one that happens to fill the width. And a
slice of a line holding a double-width character can come up a character short, because
`ctx:tint` counts columns while a Lua string counts characters and the host collapses a wide
character to one entry. `docs/plugins.md` says what each would need.

### A plugin can hold a pane still

`neotmux.pane.freeze(id)` stops the pane moving; `neotmux.pane.unfreeze(id)` lets it catch up,
and `neotmux.pane.frozen(id)` says which it is. While a pane is frozen the host draws the
screen as it was at the moment of freezing, and `pane.content`, `pane.scrollback`,
`pane.history` and `pane.revision` all answer about that same instant.

That last part is the point. A selection highlight over a pane that is still printing is a
highlight over different text a moment later, and a plugin handed the live text while the host
draws a held one has computed the wrong selection. The pane's grid, its text, its history and
its revision cannot come apart, because all four are read out of one held copy of the screen.

Nothing is lost. The child's output goes on being read from its PTY, parsed into the live
screen underneath, and dispatched as `pane_output`; unfreezing shows everything that arrived.
A resize reaches the child and the live grid and not the held view, because a child must never
be lied to about its terminal. A frozen pane whose child exits closes like any other: freezing
is not a lifeline. A second `freeze` keeps the view it already has rather than re-anchoring a
selection out from under whoever took one. The held copy costs what the pane costs — a full
10 000-line history is tens of megabytes of it.

### A plugin can ask the terminal to take something, and is told only what is true

`neotmux.clipboard.set(text)` writes an OSC 52 sequence to the client's terminal and answers
`true` when the sequence was written and flushed, or `false` and a reason when it was not.

`true` does not mean the clipboard holds the text, and neotmux cannot find that out: OSC 52
has no reply, a terminal that does not implement it discards it in silence, and one that does
may have it turned off or be behind an ssh session that dropped it. So a plugin must not put
"Copied to clipboard" on a `true` — that is the bug this return value exists to let it avoid.
tmux checks terminfo's `Ms` and writes nothing at all when it is absent; neotmux does neither
half of that, because `Ms` describes what the entry for `$TERM` claims rather than what the
terminal did, and because writing an OSC a terminal does not know is harmless.

There is still no paste buffer in the host. Yanked text goes wherever a plugin puts it.

### A plugin can measure a string in columns

`neotmux.width(text)` answers how many columns `text` takes on a pane's grid. `#text` is bytes
and `utf8.len` is characters; a wide character is one entry in a `pane.content` line and two
cells on the grid, so a tint whose width was counted either of the other two ways drifts right
by a cell per wide character before it. The rule is read out of the same code the grid is
built with, so the two cannot disagree. A combining mark adds nothing, because it is drawn
into the cell before it, and a control character adds nothing because it is never drawn.

### A read of a pane can keep its trailing blanks, and say which rows wrap

`opts.pad` on `pane.content` and `pane.scrollback` pads every row to the pane's width in
columns instead of cutting the blank rest of it. Cutting is right for reading text and wrong
for a rectangle selection, which has to know that the columns past the end of a short line are
spaces the pane is showing.

`opts.wrapped` adds a third return value: one boolean per line, saying whether that line ran
off the end of the pane and continues on the one below. It is the terminal's own flag, not a
guess — a line exactly as wide as the pane looks identical either way — and it is what tmux's
copy rule turns on: a wrapped line keeps its trailing blanks and gets no newline, an unwrapped
one gets both. A resize clears the flags, because the old wrap points no longer describe the
new width.

### `pane.history` says how many lines have fallen out of the front

It now answers two numbers: how many lines are above the screen, and how many have been
dropped from the front since the pane opened. An absolute line is `dropped + index`, which is
a number nothing renumbers — so a saved anchor keeps naming the line it was taken on, and a
plugin can see when the line it named is gone. The revision would not have told it: that moves
on any output at all, and stands still for none of the evictions.

The terminal reports no evictions, so the host counts them, by watching the lines the terminal
appends against the lines it still holds. The count is exact as long as no single read from a
pane's PTY appends as many lines as the pane holds — a read is at most 8 KiB and a pane holds
10 000 lines, so ordinary output cannot reach it. A child that sends thousands of scroll
sequences in one chunk can, and then the count is low.

### A plugin can recolour a region without erasing it

`ctx:tint(pane, x, y, w, h, colours)` sits next to `ctx:rect` and takes the same rectangle.
`rect` paints its area by writing a space into every cell, so it erases whatever was under it;
`tint` writes no glyph at all and only changes the colours of the cells that are there. That
is the visual of selecting text, and until now it could not be expressed.

A colour left out is a colour the cell keeps. `{ bg = 4 }` recolours the background and leaves
whatever coloured the text, so a tint over syntax-highlighted output keeps the highlighting;
name both and both change. There is no spelling for "put this channel back to the default" —
omitting it means keep, not reset.

A tint naming no colour raises. `rect` with no colours is a deliberate erase, but a tint with
none could only be a mistake, and the two calls are going to sit beside each other in plugin
code where the wrong one is otherwise silent.

A cell nothing has written tints like any other: it is a blank with the terminal's own
colours, and tinting it gives a coloured blank rather than a glyph.

### A plugin can read the lines above a pane's screen

`neotmux.pane.scrollback(id, opts)` answers the history above the visible screen on the same
terms as `pane.content` — lines, never bytes — with `opts.from` and `opts.lines` picking a
window out of it. `neotmux.pane.history(id)` is the cheap count beside it, the way
`pane.revision` is the cheap half of reading the screen.

**Line 0 is the oldest line the pane still holds**, and numbers increase downwards. New output
is appended and takes the next number, so a position a plugin saved last tick still means the
line it saved it for; `history()` grows and nothing is renumbered. The exception is the
10 000-line history filling up, where each new line drops the oldest and every number shifts
down by one.

Reading does not scroll the pane. `vt100` has no read path into its scrollback that does not
go through the screen's own position in it, so the read moves that position and puts it back
before answering — inside one call, on the thread that also renders, so no frame is drawn in
between and nobody watching the pane sees it move.

The revision gates this too: the history cannot change without the pane's terminal being fed,
so an unchanged revision means an unchanged history. A read costs the lines it returns, and a
call with no `lines` count is the whole history.

### A band can be taken off the left or the right

`neotmux.reserve` now takes `'left'` and `'right'` as well, measured in **columns** rather
than rows. Everything else about a reservation is unchanged: a handle back, one reservation
per source, `ctx:reserved()` answering that source's own band in surface coordinates, and
bands allocated in load order from the outside in.

`left` and `right` were deliberately absent until now, on the grounds that the consumers were
a status line and a window list and nobody wanted a vertical band. A sidebar is that consumer,
so the omission stopped being right.

The two axes are bounded separately: four rows on a horizontal edge as before, forty columns
on a vertical one. Four is where a line or two of text stops being a line of text; forty is
half of the eighty columns a terminal is still written against, past which a band is not a
margin on the panes but the layout.

They are also *refused* separately. A terminal that cannot spare the rows and still leave the
panes a grid `vt100` survives applies none of the horizontal bands — and keeps the
vertical ones, because rows and columns are spent out of different room and hiding a sidebar
over a shortage of rows would give the panes nothing back for it.

A horizontal band spans the full width and a vertical one only what the horizontal bands
left, so no two bands ever share a cell.

### A reserved band can change size

The handle `neotmux.reserve` answers now has a second method: `handle:resize_to(cells)`, the
same band on the same edge at a different size, from the next frame.

It is a resize rather than a remove and a fresh reservation because bands are laid out in the
order they were reserved, outward in: a plugin that gave its band back and asked for another
would come back inside everyone who reserved after it, so anything that folded would walk
across the screen. Resizing edits the reservation in place and keeps its place on the edge.

The refusals are the ones `reserve` already draws. Zero and a count past the edge's own bound
raise, because they are typos — there is no band of no cells, so `ctx:reserved()` never
answers a rect a plugin must not draw in. Growing into cells another plugin holds answers
`nil` plus a message. The cap is counted against the other bands on the edge only, so a band
never pays for its own cells twice.

`neotmux.pane.resize` moves a boundary by a delta, which is why this one is spelled
`resize_to`.

### A sidebar, shipped and off by default

`plugin/sidebar.lua` lists every window, every window's panes and what each pane is running,
down a reserved left band. It marks the window being shown and the focused pane. It is off
until you turn it on — `require('neotmux.defaults').sidebar = true` — like the
watcher and unlike the status bar: a status line spends a row, a sidebar spends a third of the
width.

It is an ordinary file making ordinary API calls, and it needed no new ones. A pane whose
command the host could not read shows an em dash; every other pane, hidden window or not,
shows what it is running.

### `pane.info` describes every pane, not only the ones on screen

`neotmux.pane.info(id)` now answers for any pane the host owns, so a pane found through
`window.panes()` reports its foreground command and working directory the way a pane in front
of you does. It used to sample the shown window's panes alone, and a hidden pane was
indistinguishable from one that had gone.

The restriction was about cost — two `/proc` reads per pane — and the cadence is what actually
bounds that: the facts are sampled at most every 250 ms however often they are asked for, and
sampling eight panes across eight windows instead of one is not measurable as idle CPU.

A pane whose read failed is still a table with the field absent rather than `nil`; `nil` means
no pane has that id. One pane's dead child does not disturb the others' facts.

`C-b b` folds it, when the prefix is loaded too: the sidebar puts that command in the prefix's
own table, because the sidebar is the optional plugin and the prefix is the standard one.
Folded is a one-column rail rather than nothing — the markers column, so the shape of the
session is still there to see and only the names are gone. The rail is the open sidebar with
everything that does not fit cut off, so there is no second way of drawing it.

### A plugin can ask which panes a window has

`neotmux.window.panes(id)` answers with a window's panes in layout order, for any window the
host owns, hidden or not. Until now nothing mapped a window to its panes: `pane.list()` is the
shown window's list and `window.list()` is only ids, so a plugin discovered a hidden pane by
hearing it speak. A pane that had been silent since it opened therefore did not exist as far
as any plugin was concerned — and a window opened with `window.create()` starts exactly that
way, since it raises `window_opened` and no `pane_opened`.

The order is the one `pane.list()` gives: left to right, top to bottom. A hidden window keeps
the rects it was last assigned, so it has a layout too and gets the same answer — the
arrangement it will be shown in. An id no window has answers `nil`, the way `pane.current()`
and `pane.rect()` answer nothing; a living window always holds at least one pane, so an empty
list is not something you can be told.

`pane.list()` is unchanged: it is still the panes of the session in front of the user. And
finding is all this adds — there is still no way to split, focus or resize a pane in a window
nobody is looking at.

The shipped watcher uses it. It enumerates every window's panes on each tick instead of
learning ids from `pane_output`, so an agent started in a window you are not looking at is
watched from the moment it exists rather than from the moment it first prints something. The
revision gate is untouched: a pane's lines are still built only when its terminal has been fed
since it was last read, so an idle session still reads nothing.

### The dividers meet where they cross

Four panes in a grid used to leave a hole at the crossing. Each pane draws the divider down
its own right-hand gutter and across its own bottom one, so the cell where the two gutters
meet is inside neither pane's extent and nobody drew it.

It is filled now, with the glyph the arms deserve rather than a `┼` everywhere: a cell has an
arm in a direction when a divider covers the neighbouring cell that way, so a full-height pane
beside a split column gets `├` and a wide pane above two gets `┬`, and no arm is ever drawn
into a pane that is not divided there. Working that out is `neotmux.junctions`, a pure
geometry module beside `neotmux.nav`; `plugin.borders` decides what to put in the cell, and
its five new fields — `cross`, `tee_up`, `tee_down`, `tee_left`, `tee_right` — are read every
tick like `vertical` and `horizontal`. The host learned nothing: a junction is one more cell
of `ctx:line`.

### The bar counts panes and windows the way a person does

Split, end the shell in the new pane, split again: the number on the bar used to climb — 2,
then 3, then 4 — with never more than two panes on screen. The ids were right. A `PaneId` is
allocated once and never reused, and a `WindowId` likewise, because a plugin holding id 2 must
not find that it now means something else. Showing one to a person was the mistake.

The bar shows positions instead: where the pane sits in `pane.list()`, which is in layout
order, and where the window sits in `window.list()`. Both close the gap when one goes, so a
block is numbered the way tmux numbers them. Nothing in the host changed and no id was
renumbered — `status/position.lua` is a loop over a list the bar already had.

A block now reads `1.2  nvim`: the window's position, the focused pane's position, the command.

### Panes you can move between and resize

`neotmux.pane.resize('left' | 'right' | 'up' | 'down', cells)` moves the boundary between the
focused pane and its neighbour that way, by exactly the cells asked for. One rule and no
exception: tmux grows the pane unless it is at the edge, where it silently moves the other
boundary instead, and nobody can predict that. With no neighbour that way there is no
boundary and the call is refused. The boundary moved is the nearest one above the pane, so a
pane three splits deep resizes against what it is actually touching.

Cells, not a fraction: the tree still stores an `f32`, but a resize reads its current
boundary back through the same rounding `assign` lays it down with, so ten moves of one cell
move ten cells rather than nine.

`neotmux.pane.rect(id)` answers the rect `ctx:pane_rect` answers, out of the same published
geometry, from anywhere — geometry was readable only inside a tick, so a plugin that wanted
to know which pane was to its left had to register a draw callback it did not want. `list()`
was already readable anywhere and threw the rects away; that asymmetry is gone.

The shipped prefix uses both: `C-b h/j/k/l` moves the focus to the pane whose edge faces most
of yours on that side, and `C-b r` enters a resize mode where the arrows and `h/j/k/l` move
the boundary. Both are ordinary Lua in `plugin/prefix.lua`, in the same command table the
rest of the prefix is in, and the mode is a file-local flag — the host learns nothing about
modes.

### Keystrokes that no longer wait for the frame

A keystroke used to cost a whole frame. The loop waited on one of the two things that make a
frame worth drawing — the terminal's own input — and collected each pane's output with a
non-blocking read once per pass, so a shell that echoed a character a millisecond later went
unnoticed until the next frame boundary. Measured keystroke to first byte back, typing into
bash: a median of 20 ms against a bare PTY's 0.2 ms.

Every pane's reader thread now nudges a pipe the wait watches alongside the terminal, and the
frame's work starts as soon as there is a reason for it rather than at the next deadline. The
same measurement is a median of 1.3 ms.

A frame is still 16 ms and still draws at most once. What changed is where the drawing sits
inside it: the frame waits for its reason, draws, and spends the rest of its time pumping
input rather than asleep, so a key still reaches its pane the moment it arrives and a pane's
output waits for the next frame rather than drawing this one twice.

### What a pane is doing

`neotmux.pane.info(id)` answers with the pane's foreground command and that process's working
directory — `{ command = 'nvim', cwd = '/home/you/src' }` — which is the content a status line
was missing. It is the pane's foreground process group rather than the shell neotmux spawned,
so a pane running `nvim` says `nvim` and the same pane back at its prompt says `bash`.

An unknown id answers `nil`, and a fact that could not be read is an absent field; neither
raises, because a shell exiting is not a plugin's error. The facts are sampled by the host at
most every 250 ms and a call reads the last sample, so asking on every tick for every pane
costs no syscalls. Read from `/proc`: Linux-only, and unknown everywhere else.

### A status bar, not a debug line

The shipped `status` plugin now reserves the **top** row and draws what a status bar is for:
every window on the left, the one being shown capped in colour with what its focused pane is
running beside it, and on the right that command and its working directory, each behind its
own icon. `panes 2 focus 1` is gone.

It is composed from scratch every tick, so it follows the focus, a command starting or
finishing and a `cd` with no keystroke to prompt it. The directory is written the way a shell
prompt writes it — `$HOME` as `~` — and loses its leading components when the space runs out.

`$HOME` reaches it as `neotmux.home`, the one value out of the process environment the host
publishes: the VM has no `os.getenv`, and this is the same read the `~/.config/neotmux` layer
is resolved from, so the two cannot disagree. It is `nil` where the process had no `HOME`, and
the path is then written out in full.

The colours (Catppuccin Mocha), the caps and the icons are fields on the module, read on the
tick: `require('plugin.status').caps = { left = '', right = '' }` is the whole escape for a
terminal without a Nerd Font.

### The status line is a plugin now

`neotmux.reserve('bottom', 1)` takes a band of the surface and returns a handle; the panes get
what is left, and the plugin draws into the rect `ctx:reserved()` hands it with the same
`ctx:text` it would use anywhere else. `ctx:status` and the status row that came with it are
gone — the host no longer knows what a status line is, only how to hand out rows along an
edge. The shipped `status` plugin looks the same on screen and is 20 lines of ordinary API
calls.

`top` works as well as `bottom`, two sources on one edge get a row each — outermost to
whoever asked first — and a band is outside every pane's rect, so nothing a shell writes can
reach one. An edge holds at most four rows; a terminal too small for the bands it granted
hides them for the frame and gives them back when it grows, the same way it hides a pane too
small to show.

### Panes you can see

`borders`, a third shipped plugin, rules every gap between two panes with `│` and `─`.

The host reserves the gap and the plugin draws in it, the same bargain as a reserved band:
`neotmux.opt.pane_gutter` says how many cells to leave between panes, and the host never
learns what anyone puts there. Without it a divider had nowhere to go but on top of a cell a
shell was using — a vertical split of an 80-column terminal silently ate the 40th character
of every line in the left pane. Still no adjacency API: with a gutter of `g` a pane's right
neighbour starts at `r.x + r.w + g`, so `pane_rect` and `line` remain the whole story.

`ctx:line` now takes an optional glyph, so the character is the plugin's choice rather than
the host's. Omit it and you get the `#` it always drew.

### A prefix you can change

The shipped prefix is now `C-b`, with `v` splitting vertically, `-` horizontally and `o`
walking to the next pane. Its command table is a field of the module rather than a file-local,
so a user's `init.lua` can add, replace or drop a binding — `prefix.commands['%']` back to
tmux's is one line — instead of only being allowed to move the prefix key.

## v0.1.0 — 2026-09-20

The line at which a plugin author can exercise the ABI. **Not** the line at which neotmux is
a daily driver: there is no detach/attach, and that is the intended trade — the ABI is the
part that is expensive to change after publication, so it meets other people before it
freezes, and the server/client split happens against an ABI that has been used rather than
one that has not.

### What it is

A terminal multiplexer whose core is pane management and whose everything else is a Lua
plugin. tmux is fast but has no in-process extension ABI, so "install a plugin" can only mean
"run something that mutates your config". zellij has a real plugin system but pays for it
with a sandboxed WASM interpreter on the render path. nvim's answer is to trust the plugin,
run it in process, and give it a real ABI — and neotmux adds the piece nvim lacks: **a plugin
tick is a best-effort operation that is allowed to fail.**

### The host has no plugin concept

It provides an API; a loading convention decides what gets loaded; a plugin is a bundle of
API calls and is a plugin only as a result. Making that true is most of what this release is.
Identity is the chunk a piece of code was written in, and attribution is lexical everywhere —
where the calling code was *written*, never where it runs from — so a misbehaving plugin can
be named whether it fails during load, a tick, a key, an event, or inside a coroutine.

Five things had to go for it to be true: the plugin identity itself, a warning that loaded
code ought to register something, a rule that isolated only modules named `plugin.*`, a
reserved prefix key, and three events plus a status hook the binary could never fire.

Removing the prefix reservation is what proved the ABI was short a piece: a prefix could only
intercept keys it had already bound, so it was not modal and it starved every other plugin of
those keys. `neotmux.on_key` closed that, and the prefix now ships as a plugin that binds
nothing.

### A tick is allowed to fail

A global Lua debug hook samples every 1000 instructions. A tick that passes its budget is
unwound; that source's frame and the ops it pushed are discarded, the overrun is logged
against its name, and five in a row park its tick registrations — handed back by
`neotmux.resubscribe(name)`, because the alternative to recovery is restarting neotmux, which
kills every shell you are running.

What the hook cannot cut off it still attributes: a tick that spends 189 ms inside one
`string.rep` is named and counted on the way out, and parked if it keeps doing it. Being
unable to interrupt is acceptable; being unable to attribute is not.

### Panes

A binary split tree, one PTY and one grid per pane, and splits, focus, resize and close
driven entirely from Lua. The host binds no key and has no prefix: `C-a %` is a plugin.

### The ABI

`on_tick`, `on_event`, `on_key`, `keymap.set`, `neotmux.pane.{list,current,split,focus}`,
immediate-mode drawing through `ctx`, `opt`, `log`, `notify`, `quit`, `resubscribe`.
Documented in `docs/plugins.md`, with three working plugins in `examples/`.

### Not in this release

detach/attach (v0.2), scrollback and copy-mode, mouse, multiple sessions, a plugin manager,
`pane.close` and `pane.resize`, and tmux compatibility in any form.

### Install

```sh
make install            # cargo install plus the shipped runtime layer
```

`cargo install` alone copies only the binary; the default status line and prefix live in a
runtime layer that needs its own step. A C compiler is required — `mlua` builds Lua 5.4 from
vendored source. Nothing else is.

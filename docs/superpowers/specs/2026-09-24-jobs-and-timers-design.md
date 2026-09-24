# neotmux — jobs and timers

Date: 2026-09-24
Status: proposed
Scope: two host primitives that let a plugin learn something the host did not hand it — an
external process, and the passage of time. Issue #13.

## 1. Goal and non-goals

### Goal

A plugin can see what is in the panes and nothing else. `io` is not loaded, `os.execute` is
removed, and the only clock is `os.time`, which counts whole seconds. So a status band cannot
show the branch it is on, a watcher cannot poll anything while the session is detached, and
nothing can make a sound when a pane stops asking for input.

Two primitives close that, both callback-shaped so that nothing blocks a frame:

```lua
neotmux.job.start(cmd, opts)   -- run a program, get its output back
neotmux.timer.every(ms, fn)    -- be called again later
neotmux.timer.after(ms, fn)
```

Everything they are for is a plugin: a sound on a state change, a git status in a band, a CI
poll, a log watcher. This spec ships no such plugin. It ships the two calls and the rules they
keep.

### Non-goals

- **A pty.** A job is pipes. A program that wants a terminal is a pane; that is what panes are.
- **Synchronous waiting.** There is no `:wait()`. The VM runs on the server's own thread, so a
  blocking wait stops every session's frame. nvim can afford `vim.system():wait()`; this cannot.
- **A sandbox.** A plugin already runs in-process and can reach the server's own state. Being
  able to start a process changes what that is worth stealing, not who is trusted. A guard, if
  one is ever wanted, is a Lua hook over `job.start` in the same shape `on_write` already has —
  and it can be added later without changing anything here.
- **A scheduler.** A timer fires on a frame. It is not a real-time facility and does not pretend
  to be one (§5).

## 2. What it looks like from Lua

### 2.1 `neotmux.job`

```lua
local job, why = neotmux.job.start({ 'git', 'status', '--porcelain' }, {
  cwd = '/path/to/repo',           -- default: the server's own cwd
  env = { GIT_PAGER = 'cat' },     -- added to the server's environment, not a replacement
  buffered = true,                 -- default true
  on_stdout = function(lines) end, -- buffered = false only
  on_stderr = function(lines) end, -- buffered = false only
  on_exit = function(code, stdout, stderr, err) end,
})

job.id                             -- a number, stable for the life of the job
job:kill()                         -- true if it was running, false if it had already finished
```

`cmd` is a list: the program and its arguments, each one its own element. There is no string
form and no shell — a plugin that wants a shell asks for one by name (`{ 'sh', '-c', '…' }`)
and owns the quoting, the same bargain `pane.send` makes.

`buffered = true`, the default, collects the whole of each stream and hands it to `on_exit` as
two strings, unsplit. `on_stdout` and `on_stderr` are not called. This is the shape for a
program that runs and stops.

`buffered = false` calls `on_stdout` and `on_stderr` as output arrives, with **a list of
complete lines and nothing else**. A chunk that ends mid-line is held by the host until the rest
of it arrives; whatever is still held when the stream ends is flushed as one last line before
`on_exit`. A caller never joins a fragment to its remainder, because a caller is never handed a
fragment. `on_exit`'s `stdout` and `stderr` are `nil` in this mode — the caller has already seen
every line, and keeping a copy would be a second megabyte nobody asked for.

This is a deliberate departure from nvim, whose `on_stdout` hands over "the lines, and the last
one may be a fragment" and leaves the rejoining to the plugin. The API's shape is nvim's — the
`buffered` switch, the callback names, the handle — because that is what a plugin author will
already know. The fragment convention is not, because it is the part of that API that people
get wrong.

`on_exit(code, stdout, stderr, err)`:

| `code` | `err` | what happened |
|---|---|---|
| number | nil | the program exited; `code` is its status, zero or not |
| nil | string | the job never ran or was stopped: spawn failed, `kill()`, output limit, budget |

`err` is one line of English, suitable for a notify. Every ending a plugin can still be told
about calls `on_exit` exactly once, `kill()` included — a caller that has registered a callback
hears the end of the thing it started, whatever ended it. Three endings tell nobody, and all
three for the same reason: there is nobody left to tell. A job whose *source* went away, because
its chunk failed to load or was reloaded, is killed in silence — the function that would hear it
is the thing that is gone. A server told to stop (`SIGTERM`, `SIGINT`, `SIGHUP`) puts its
sessions down together rather than one at a time, so the process groups go on the way out and
nothing is called. And a `SIGKILL`ed server does not even kill them: nothing runs at all, so
every job it had keeps running, its process group along with it.

`start` answers `(nil, message)` rather than raising when it refuses: an unreadable `cwd`, a
program that is not there, too many jobs already (§6). Two values, the shape `keymap.set`
already uses.

### 2.2 `neotmux.timer`

```lua
local t = neotmux.timer.every(5000, function() ... end)
local once = neotmux.timer.after(200, function() ... end)

t:stop()   -- true if it was running, false if it had already stopped or fired
```

`ms` is a whole number of milliseconds, clamped up to the frame interval (§5). `after`'s
callback runs once and the timer is then stopped; `every`'s runs until stopped.

Both answer `(handle, nil)`, or `(nil, 'the session is ending')` for an arm from inside the
`on_exit` a session teardown gave — the same refusal `start` has, and for the same reason: that
timer would belong to a session with no frame left to fire it, and nothing would ever drop it.

### 2.3 Options

Two new entries in `neotmux.opt`, both host-side numbers with the same shape as
`tick_budget_ms`:

| option | default | what it bounds |
|---|---|---|
| `job_output_bytes` | `1048576` | what one job may collect or hold before it is killed |
| `job_concurrency` | `8` | how many jobs one source may have running at once |

## 3. Where the work happens

### 3.1 The path bytes take

There is exactly one precedent in this codebase for getting bytes from somewhere else into the
VM, and a job is the same problem: a pane's child writes, a reader thread picks it up, hands it
down a bounded channel, and hits the server's `Waker` so the `poll` wakes; the frame drains the
channel and turns it into something the VM is told about.

A job reuses that path whole:

```
child stdout/stderr ─ reader thread ─ SyncSender(bounded) ─┐
                                                           ├─ Waker::wake() ─ poll wakes
                        the frame drains it ───────────────┘
```

The `Waker` is `Clone + Send` and one pipe already serves every pane reader, so a job adds **no
file descriptor to the poll set** and needs no change to the loop's slot bookkeeping. That is
the reason to copy this path rather than register the child's pipes with `poll` directly.

`pty.rs`'s `read_loop` is already generic over `R: Read`; a child's `ChildStdout` satisfies it.
What a job cannot reuse is `PtyHandle` itself, which is a pty, and `CommandBuilder`, which is
portable-pty's. A job is `std::process::Command` with `Stdio::piped()`.

### 3.2 When a callback runs

Callbacks run inside the frame, **before `dispatch_tick`**. A tick therefore sees the state its
own plugin's callbacks have already updated, in the same frame the output arrived. There is no
`vim.schedule` equivalent and no fast-event context: a callback can touch anything any other
callback can touch, because it runs in the same place.

Order within a frame:

```
drain job output and exits  → job/timer callbacks
events (pane opened, …)     → dispatch_event
                              dispatch_tick
                              drain_ops
```

### 3.3 Not an `Event`

Job completion does **not** become a `neotmux_proto::Event`. That type is the wire's — it is
matched exhaustively by the encoder and travels to a client that no longer reads it — and a
process that lives and dies inside the server has no business there. Jobs and timers keep their
own table in the Lua subsystem and their callbacks are called directly, not dispatched through
`Kind::Event`.

### 3.4 The id is minted in Lua

An op cannot answer. `job.start` needs to hand back a handle in the same call, and the spawn
itself is the host's, so the id is chosen on the Lua side before the op carrying it goes out —
the way `window.create` already mints a window id. `job.id` is that number.

## 4. Attribution, budget and what happens when a callback misbehaves

**Attribution.** A job and a timer belong to the chunk that called `start`/`every` — the caller,
the way an op is attributed, not the callback's definition site. A plugin that hands out a
helper which starts a job on its behalf is the owner of that job.

**Budget.** One callback invocation is one budgeted region, with the tick budget (8ms by
default). Jobs are independent of one another, so they are not grouped by source the way tick
subscriptions are.

**Overrun.** What is stopped is the thing that keeps calling: a timer whose callback overruns
`tick_overrun_limit` times in a row is stopped; a job whose callback does is killed, and its
`on_exit` is told why. Neither touches the source's other registrations. The existing park — the
one that pulls a source's tick subscriptions out of the registry and gives them back on
`neotmux.resubscribe` — is not used here. A plugin whose timer runs away has not shown that its
drawing is broken, and stopping its drawing would be a punishment for the wrong thing.

A stopped timer and a killed job are not restored by `resubscribe`. The plugin starts another
one, or does not.

## 5. Time

A timer fires on a frame and on nothing else. The frame is 16ms, so:

- an interval below 16ms is clamped up to it, and `timer.every(1, …)` means "every frame";
- a timer fires **at most once per frame**. A timer that should have fired five times while the
  VM was busy fires once, not five times. There is no catch-up burst, and a plugin that needs to
  know how much time really passed asks the clock;
- the frame is an upper bound, not a lower one. The loop cuts a frame short when a pane speaks,
  so a 16ms timer under a chatty pane fires more often than a 16ms timer under a quiet one, and
  a 5000ms timer fires at the first frame at or after 5000ms.

This is a coarse facility and the documentation says so. What it is for is "every few seconds,
go and look at something", which is what every plugin that wants a timer actually wants.

A timer keeps the server doing work while nothing is attached, which is the point — a watcher
that stops watching when you detach is not a watcher — but it also means a plugin can keep an
idle machine awake. That is the plugin's choice to make, and the option to make it differently
is to not register the timer.

## 6. Limits

**Output.** A job may hold `job_output_bytes` (1MiB by default) at once: in `buffered` mode the
total collected, in streaming mode whatever has not yet been handed over plus the partial line
being held. Exceeding it kills the job and calls `on_exit(nil, stdout, stderr, 'output limit')`
with what had been collected — which in streaming mode is `nil`, since those lines were handed
over as they arrived. A plugin that expects more output than that is asking the wrong
primitive for it.

**Concurrency.** A source may have `job_concurrency` (8) jobs running at once. The ninth `start`
answers `(nil, 'too many jobs')`. This is not a security boundary — it is the thing that turns
a loop in a plugin into a message instead of a fork bomb.

**Timers** are not capped. A timer costs a callback per frame at worst, which the budget already
bounds, and the overrun rule already stops one that abuses it.

## 7. Lifetime

**A job and a timer belong to their source.** Unloading or reloading a plugin kills its jobs and
stops its timers — the same sweep that already drops a source's registrations, reservations and
floats, with one more line for this.

**They also belong to a session.** A callback runs with that session as its ambient one, because
a callback that calls `neotmux.notify` or `pane.send` has to have somewhere to send it. The
session is the one that was current when the job or timer started — or, when there was none, the
first session whose frame runs afterwards. That second case is not an edge: a plugin's `init.lua`
runs before any session exists, and arming a poll at load is the most ordinary thing a plugin
with a timer does. When that session is gone, the job is killed and the timer stopped.

**Server shutdown** kills every job without waiting for it, the way a pane's child is hung up.

**A job that outlives its plugin's interest** is the plugin's problem: `kill()` is there, and a
job whose callbacks the plugin no longer wants is one the plugin should kill.

## 8. Failure, and what is said about it

| what | who hears about it |
|---|---|
| spawn failed (no such program, bad `cwd`) | `start` answers `(nil, message)`; nothing is notified |
| the program exited non-zero | `on_exit(code, …)`; the plugin decides whether that is news |
| output limit, budget, session gone | `on_exit(nil, …, err)`; the host also notifies for the limit and budget cases, because those are neotmux's decisions rather than the program's |
| plugin unloaded or reloaded | nobody: the callback that would hear it belongs to the chunk that is gone, so the job is killed in silence |
| server told to stop (`SIGTERM`, `SIGINT`, `SIGHUP`) | nobody: the sessions go down together rather than one at a time, so the jobs are killed with no ending delivered |
| server `SIGKILL`ed | nobody, and nothing is killed either: no code runs on the way out, so the jobs and their process groups outlive the server |
| a callback raised | reported the way every other raising callback is, attributed to its source; the job keeps running |

A raising callback does not kill the job. A callback that raises on one chunk of output may well
handle the next one, and a plugin that wants to stop calls `kill()`.

## 9. Testing

The VM-level tests take the same shape the existing subsystem tests do — a `LuaSubsystem` built
over a temp layer, keys and ticks driven directly — with a fake in place of the process:

- **A job runner seam.** The thing that spawns and reads is behind a trait, the way `Clipboard`
  already is, so a test can hand back scripted output and an exit code with no process and no
  timing. The real implementation is the one described in §3.1.
- **A clock seam.** Timers take the frame's `Instant`, which the test supplies. `PaneInfoCache`
  already does exactly this, so the precedent is in the host.

What is tested: buffered output arrives whole; streaming hands over complete lines only and
flushes the held fragment at the end; `on_exit` is called exactly once on every path, including
`kill`, the output limit, the concurrency refusal path (which does not start a job at all) and a
dead session, while an unloaded plugin's jobs are killed in silence; a timer fires at most once per frame; `after` fires once; a
timer stopped inside its own callback does not fire again; overrun stops the timer and kills the
job and leaves the source's tick subscriptions alone; a raising callback leaves the job running.

An end-to-end test runs one real process — `/bin/echo` — to prove the seam is wired to something
real. Everything else is the fake.

## 10. What this unlocks, and what it does not

With these two, a plugin can: make a sound when `neotmux.state` says a pane went idle or
blocked; put a branch, a CI state or a battery in a band; poll a queue while detached; watch a
log with `buffered = false`.

It does not, on its own, give a plugin: a way to read a file (`io` is still gone — `cat` is a
job, which is enough and is honest about the cost); an environment to read (`os.getenv` is still
gone, and a job inherits rather than composes); a way to talk to something over a socket. Those
are separate questions, and none of them is blocked by this.

# neotmux — a write to a pane passes a hook

Date: 2026-09-24
Status: proposed
Scope: one host primitive — every write of text into a pane is offered to Lua first, and a
handler that refuses it, or fails to answer, stops it. The rules that decide are Lua's.

## 1. Goal and non-goals

### Goal

An agent that drives a program through a neotmux pane — `neotmux send %4 '…' --enter`, or a
plugin's `neotmux.pane.send` — puts every byte through the multiplexer before it reaches the
pty. That makes neotmux a checkpoint no agent-side mechanism is: it sits outside the agent, it
is the same for every agent, and a rule written as code gives the same verdict for the same
input every time — unlike asking a model to be careful.

Today nothing can see those bytes. §3 of `2026-09-23-the-command-surface-design.md` says a host
command is *"answered without the Lua layer being consulted, entered or woken"*, and `send` is a
host command.

This spec adds the one thing that is missing and nothing else:

```lua
neotmux.on_write(function(write)     -- { pane =, text =, from = 'tool' | 'plugin' }
  if write.text:find('rm%s+%-rf') then
    return false, 'rm -rf is blocked'
  end
end)
```

**What blocks what is not in the host.** A guard is a Lua convention on top of this hook, the
way `neotmux.mode`, `neotmux.box` and `neotmux.prompt` are conventions on top of floats and
keys: the pattern table, the policy per `from`, the wording of a refusal — all of it lives in
`runtime/lua/neotmux/guard.lua` and in a user's own layer, and the host has never heard of any
of it. The host owns two things only, because the Lua that failed cannot own them: turning a
handler's failure into a refusal, and knowing which kind of connection asked.

### Non-goals

- **The human's keystrokes.** A key typed at an attached client is not a write here. Guarding
  what a person types is `on_key`'s job (#22) — the text is on the screen and has to be guessed
  at, which is a different problem with a different answer. This hook covers writes whose text
  is known exactly.
- **Rewriting.** A handler allows or refuses. It cannot change the text. Two handlers that each
  rewrote would make "what was sent" and "what arrived" different things, and an audit trail
  that cannot be read is worse than none.
- **Stopping an agent that does not use neotmux.** An agent running commands through its own
  shell tool bypasses this entirely. Enforcing "panes only" belongs to the agent's
  configuration or a sandbox, and the docs say so rather than implying coverage that is not
  there.
- **Guarding reads.** `capture` and `pane.content` are not writes and are not hooked.

## 2. The primitive

`neotmux.on_write(fn)` is a subscription like `on_key` and `on_tick`, registered the same way,
owned by the source that registered it, and counted in the same registry (`KindTag::Write`).

The handler is called with one table:

| field | what it is |
| --- | --- |
| `pane` | the pane the text is going to |
| `text` | exactly the bytes that would be written, as a string |
| `from` | `'tool'` for a connection that is not a client, `'plugin'` for a plugin's own `pane.send`. A person's keys are not writes (§1), so `'terminal'` is not emitted today. |

and answers:

| return | meaning |
| --- | --- |
| nothing, `nil`, or `true` | this handler does not object |
| `false` | refused, with no reason given |
| `false, 'why'` | refused, and `why` is what the caller is told |

Every registered handler runs, in registration order, until one refuses. The first refusal is
the answer and the rest are not called — a refusal is a decision, not a vote.

## 3. What passes through it

Two paths write text into a pane, and both are hooked:

- `neotmux send <pane> <text>` — the host command, `from = 'tool'` on a command connection.
- `neotmux.pane.send(id, text)` — a plugin's own write, applied with the frame's ops,
  `from = 'plugin'`. Hooking this is what stops plugins being the bypass, and it means a
  plugin's write is offered to every *other* plugin's handler as well as its own.

Not hooked, deliberately: a key from an attached client (§1), a pty resize, and anything the
host writes for its own reasons (a query answer, a paste the terminal itself sent).

## 4. Where it runs, and what it costs

**At the door, inline.** `answer()` runs inside `door()`, which the frame calls before it ticks
any session, so the VM is idle at that moment: the hook is dispatched there and then, and the
connection is answered in the same breath. No frame is entered and nothing is queued.

That matters because the alternative — queueing the write for the next frame — adds up to a
whole frame (16 ms) to every agent write, which is three orders of magnitude more than the
work. A Lua pattern over a line of text is microseconds; the routing is what would be
expensive, so the routing is what this avoids.

**With no handler registered, nothing changes.** The registry knows how many `Write`
subscriptions exist without running Lua, so a server whose layers register none answers `send`
exactly as it does today — at the door, in Rust, with the Lua layer untouched. The path only
becomes the one described here once something is listening.

For a plugin's own `pane.send`, the hook runs when the op is applied, which is after the tick
that raised it has returned — so the VM is idle there too, and a handler is never re-entered
from inside the tick that queued the write.

## 5. Failure is a refusal

For `send`, `2026-09-23-the-command-surface-design.md` §5's rule is that when Lua fails the
built-in answers. **This spec inverts that rule for this hook, and only for it.** A write whose
handlers did not all say yes is not written:

| what happened | the answer |
| --- | --- |
| a handler returned `false` | refused, with its reason |
| a handler raised | refused, naming the source that raised |
| a handler overran its budget | refused, naming the source that overran |
| the VM could not be entered at all | refused |

The reason is the one thing a guardrail must not do: fail open. A `kill` that still works when
the Lua layer is broken is a control CLI doing its job; a `send` that goes through unchecked
because the check crashed is the check being useless exactly when something is wrong.

The cost is stated plainly rather than hidden: **once a handler is registered, a broken runtime
layer stops agent writes.** That is the trade the registration is, and it is why registration —
not the host's own opinion — is what switches the behaviour on.

**The budget** is one interval for the whole dispatch, the way `dispatch_key`'s is, not a slice
per source: the path has to stay fast, and the failure is already a refusal, so there is nothing
to gain by metering each handler on its own clock. An overrun still refuses the write and names
the handler that was running when it did, so a slow handler costs its write, not the server, and
a person can still tell which layer is slow.

## 6. Which connection asked

The socket already distinguishes the two: a connection that sent `Attach` is a client with a
terminal; one that sent `Command`, `List` or `Kill` is not. `from` carries that distinction to
the handler as `'tool'` for the second kind — `'terminal'` is reserved for it, not emitted,
because a person's keys are not writes (§1) — and as `'plugin'` for a plugin's own `pane.send`,
which is not a connection at all. A policy can still be written as "an agent may not, a person
may" without the host deciding what an agent is; it just never sees a `'terminal'` write to
decide about.

**The rule this establishes:** a command that changes what neotmux itself does — reloading the
config, evaluating Lua, adding or removing a subscription, changing `opt` — is answerable to a
`terminal` connection only, and a `tool` connection is refused it. Nothing in the tree is such
a command today. The rule is written now, and the connection kind is plumbed now, so that the
first one to arrive — stage 4's plugin commands are the likely first — cannot be added without
a decision about which side of this line it sits on.

What this does not defend against is an agent with a filesystem: a runtime layer is a file, and
an agent that can write files can write a layer that registers nothing. That is §1's last
non-goal, and the docs say it.

## 7. What a refusal looks like

Three places, because three audiences:

- **The caller.** `neotmux send` exits non-zero with the reason on stderr, the way every other
  refusal in the command surface does. `pane.send` answers `false, why`, which is the shape it
  already has for a pane the session does not hold.
- **The person.** A notification, raised by the host: `write to %4 refused: rm -rf is blocked`.
  What an agent was stopped from doing is something the human at the terminal should see happen,
  not something they find in a log later.
- **The log.** The same sentence, with the source that refused, at `warn`.

## 8. The Lua side of it

`neotmux.guard` ships as a convention, not as a host feature:

```lua
-- runtime/lua/neotmux/guard.lua, in the same spirit as neotmux.mode and neotmux.box
local guard = require('neotmux.guard')

guard.block('rm%s+%-%w*[rf]', 'rm -rf is blocked')
guard.block('git%s+push%s+.*%-%-force', 'a force push is blocked')
guard.only('tool')            -- these apply to agents, not to the person
```

It registers one `on_write` handler and holds the tables. A user replacing it with their own
module changes nothing about the host, which is the point of it being a convention.

## 9. Testing

- **The hook, at the Lua layer**: a handler sees the text exactly; a `false` refuses; a refusal
  short-circuits the handlers after it; `from` is what the connection was.
- **Fail closed**: a handler that raises refuses the write; a handler that overruns its budget
  refuses it; and in both cases the pty was not written to — asserted by reading the pane, not
  by trusting the return.
- **No handler, no change**: with nothing registered, `send` is answered at the door and the
  Lua layer is not entered — asserted by a plugin that would log if it were ticked.
- **Both paths**: the same handler refuses a CLI `send` and a plugin's `pane.send`.
- **End to end, on a real pty**: an agent's `neotmux send` of a blocked line leaves the shell
  with nothing typed, exits non-zero, and the human's screen carries the notification.

## 10. Staging

1. **The hook and the CLI path.** `on_write`, `KindTag::Write`, dispatch at the door, fail
   closed, and `from`. At the end of it `neotmux send` can be refused by a plugin.
2. **The plugin path.** `Op::PaneSend` runs the same hook at apply time.
3. **The convention.** `neotmux/guard.lua`, the shipped module, and the docs — including the
   sentence about what this does not cover.

Stages 1 and 2 are the mechanism; stage 3 is what a person actually writes.

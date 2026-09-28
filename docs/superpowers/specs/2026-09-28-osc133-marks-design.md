# neotmux — a pane's commands, from the shell's own marks

Date: 2026-09-28
Status: proposed
Scope: the OSC 133 half of #15. The host keeps the marks a shell sends, and `neotmux.pane.marks`
reads them. A Lua convention, `neotmux.commands`, turns them into commands. OSC 8 links are the
other half of #15 and are not here.

## 1. Goal and non-goals

### Goal

A terminal sees text. It cannot tell where a prompt ends, where a command's output starts, or
how the command exited. A shell that sends OSC 133 says so in the stream:

| mark | meaning |
|---|---|
| `ESC ] 133 ; A ST` | a prompt starts here |
| `ESC ] 133 ; B ST` | the command line the user types starts here |
| `ESC ] 133 ; C ST` | the command's output starts here |
| `ESC ] 133 ; D [; exit] ST` | the command finished, with that exit status |

`ST` is `BEL` or `ESC \`. After this change the host keeps each mark against the line it
landed on, in the same numbering `neotmux.pane.history` uses. A plugin can then ask a pane for
its commands: where each one's prompt, input and output are, whether it is still running, and
how it exited.

### Non-goals

- OSC 8 hyperlinks. Tracking a link means tracking a range of cells as they are written, which
  is a different job from noting where a cursor was. It stays open on #15.
- Anything drawn. There is no gutter, no fold and no jump key here. They are plugins for later,
  and they read what this adds.
- Setting up the user's shell. The docs carry a bash and a zsh snippet, and nothing is
  installed.
- A `command_finished` event. A plugin reads `neotmux.commands` on its tick, the way every
  shipped plugin reads pane state today.

### Nothing breaks without marks

This is a hard requirement, not a nicety.

- A pane that never sends a mark behaves exactly as it does today, and `marks` answers an empty
  list.
- OSC 133 is already swallowed. `vt100` hands it to `Callbacks::unhandled_osc` and draws
  nothing (`vt100-0.16.2/src/perform.rs:234`). This change only starts listening.
- Marks that stop partway, such as `ssh`, `bash --norc` or a container shell inside a shell
  that marks, never leave a command running forever (§4).

## 2. Where a mark lands

A mark is kept as `{ line, col, kind, exit }`:

- `line` is absolute: `dropped + index`, where the history's lines come first and the screen's
  rows follow, the numbering `neotmux.pane.history` and `plugin.stamps` already use.
- `col` is the cursor column.
- `kind` is `A`, `B`, `C` or `D`.
- `exit` is the status, only for a `D` that carried one.

The cursor does not move inside an OSC string. So at the moment `vt100` calls `unhandled_osc`,
the cursor is where the mark was written, and its row plus the line number of the screen's top
row is the mark's line.

The top row's number is `dropped + history len`. It is only known **between** feeds, because
`dropped` is counted per feed from the scrollback offset (`screen/evictions.rs`). A callback in
the middle of a chunk would see a history that has grown by lines not yet counted.

So `TermState::feed` splits the chunk just before every `ESC ] 133 ;` it contains. Each piece is
fed the way a whole chunk is today, and the top row's number is set on the callbacks before the
piece goes in. A piece starts with the mark, so the callback fires before the piece has scrolled
anything, and the number it holds is right.

An OSC split across two chunks (`ESC ] 13` | `3;A BEL`) needs nothing extra. The callback fires
in the second chunk, before anything else in it, and the top row has not moved since the first
chunk ended.

Splitting costs one `memmem` per chunk, plus one more feed per mark, and a shell sends about
four marks per command.

**The alternate screen keeps no marks.** A mark that arrives while `screen.alternate_screen()` is
true is dropped. A TUI's rows are not the shell's lines, and a nested multiplexer passing its
own shell's marks through must not rewrite the outer shell's structure.

## 3. What the host keeps

Each pane's `TermState` holds its marks in line order, and they obey four rules:

- **A prompt replaces what is below it.** A new `A` at line `L` removes every mark on a line after
  `L` before it is added. A shell redrawing its prompt, whether after a resize or after `clear`,
  writes its `A` again where the old one was, and whatever was below it has been overwritten.
  Marks on line `L` itself stay: a command that printed nothing leaves its `C` and `D` on the
  very line the next prompt is drawn on.
- **One mark of a kind per line.** A second mark of the same kind on the same line replaces the
  first, which is what takes an old `A` or `B` off a line a prompt was redrawn on.
- **Marks go with their lines.** When the history drops lines, the marks on them go too, as
  `plugin.stamps`' times do.
- **Bounded.** Marks are pruned to the lines the pane still holds, so a pane never holds more
  marks than about four per line of history.

`PaneRead` gains `marks(pane, first, last) -> Option<Vec<Mark>>`, answering the marks whose
line is in `first..=last`. `neotmux_proto::screen::Mark` is `{ line: u64, col: u16, kind:
MarkKind, exit: Option<i32> }`.

## 4. What Lua sees

```lua
neotmux.pane.marks(id)            -- every mark, oldest first, or nil for no such pane
neotmux.pane.marks(id, first, last)
-- { { line = 1203, col = 0, kind = 'prompt' }, { line = 1203, col = 28, kind = 'input' },
--   { line = 1204, col = 0, kind = 'output' }, { line = 1210, col = 0, kind = 'done', exit = 0 } }
```

`kind` is spelled for readers: `prompt`, `input`, `output`, `done`.

`neotmux.commands` is a convention module in `runtime/lua/neotmux/commands.lua`, beside
`neotmux.state`. It holds the rules for marks that stop partway:

```lua
local commands = require('neotmux.commands')
commands.of(id)   -- oldest first:
-- { { prompt = 1203, input = 1203, output = 1204, done = 1210, exit = 0, state = 'done' }, ... }
commands.last(id) -- the newest one, or nil
```

- A command starts at a `prompt`, and any of `input`, `output`, `done` and `exit` may be missing.
- `state` is one of four values:
  - `done` when a `done` mark closed it.
  - `running` when it has an `output` mark and no `done`, and it is the newest command.
  - `prompt` when it is the newest command and has no `output` yet: the shell is waiting at its
    prompt, and nothing has been run.
  - `unknown` when a newer prompt arrived before any `done`. That is the "marks stopped" case:
    the shell that would have sent `D` is not the one that sent the next `A`, or nothing was
    run at all.
- An `output` with no `done` is only `running` while it is the newest command. There is no
  timeout: a long build is running for as long as it runs.
- A `done` with no `prompt` before it, left over from a history that dropped its prompt, starts
  no command and is skipped.

## 5. Shell setup (docs only)

`docs/plugins.md` gets a subsection with the two snippets below, which a user adds to their own
rc file:

```bash
# bash
PS0='\e]133;C\a'
PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
PROMPT_COMMAND='printf "\e]133;D;%s\a" "$?"'"${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
```

```zsh
# zsh
precmd()  { print -Pn "\e]133;D;$?\a\e]133;A\a" }
preexec() { print -n "\e]133;C\a" }
PS1="$PS1"$'%{\e]133;B\a%}'
```

The bash `D` goes first in `PROMPT_COMMAND`, so `$?` is the command's status and not the status
of another hook. The first `D` a shell sends has no command before it, and §4 skips it.

## 6. Testing

- **`neotmux-pty`, fed bytes.** Test these cases:
  - a mark at the start of a chunk and one in the middle, with output before it that scrolled
    the screen
  - a mark split across two chunks
  - `D;0`, a bare `D`, and a malformed exit
  - history overflow dropping marks with their lines
  - the alternate screen keeping none
  - a new `A` replacing what is below it, and keeping the marks on its own line
  - both `ST` forms, `BEL` and `ESC \`
- **`neotmux-lua`.** `neotmux.pane.marks` answers with the spellings and the range. It answers
  nil for an unknown pane and `{}` for a pane with no marks. `neotmux.commands.of` returns
  `done`, `running`, `prompt` and `unknown`, and skips a leading `done`.
- **By hand.** Start bash with the snippet inside neotmux and run `true`, `false` and
  `sleep 5`. A key handler logs `commands.of` for the pane. While `sleep` runs, the last
  command is `running`. Once it finishes it is `done` with exits 0, 1 and 0. Then `ssh` or
  `bash --norc` in, run something, and exit. The inner command is not left `running`.

## 7. Docs

`docs/plugins.md`: `neotmux.pane.marks` in the panes list and its paragraph, "The commands
convention" section, and the shell snippets. `CHANGELOG.md`. A comment on #15 saying the OSC
133 half is done.

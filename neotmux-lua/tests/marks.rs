//! A shell's OSC 133 marks, read back through `neotmux.pane.marks`, and the commands
//! `neotmux.commands` makes of them.

mod support;

use support::{Built, built, feed, keys, logged, pane};

/// A key that logs every mark of pane 1, then what an unknown pane answers.
const MARKS: &str = "neotmux.on_key(function(key)
  if key ~= 'M' then return false end
  for _, m in ipairs(neotmux.pane.marks(1)) do
    neotmux.log.info(m.kind .. '@' .. m.line .. ':' .. m.col .. (m.exit and ('=' .. m.exit) or ''))
  end
  neotmux.log.info('in 1..1: ' .. #neotmux.pane.marks(1, 1, 1))
  neotmux.log.info('unknown: ' .. tostring(neotmux.pane.marks(99)))
  return true
end)";

/// A key that logs every command of pane 1 in one line each.
const COMMANDS: &str = "local commands = require('neotmux.commands')
neotmux.on_key(function(key)
  if key ~= 'C' then return false end
  for _, c in ipairs(commands.of(1)) do
    neotmux.log.info(c.state .. ' ' .. c.prompt .. ' ' .. tostring(c.output) .. ' ' .. tostring(c.exit))
  end
  local last = commands.last(1)
  neotmux.log.info('last ' .. (last and last.state or 'none'))
  return true
end)";

fn more(sub: &Built, text: &[u8]) {
    sub.terminals().feed(pane(), text);
}

#[test]
fn a_panes_marks_are_read_back_with_their_spellings() {
    let (_dir, sub) = built(MARKS);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;2\x07",
    );
    keys(&sub, &["M"]);
    assert_eq!(
        logged(&sub),
        vec![
            "prompt@0:0",
            "input@0:2",
            "output@1:0",
            "done@2:0=2",
            "in 1..1: 1",
            "unknown: nil",
        ]
    );
}

#[test]
fn a_shell_that_sends_no_marks_has_none_and_no_commands() {
    let (_dir, sub) = built(&format!("{MARKS}\n{COMMANDS}"));
    feed(&sub, b"$ ls\r\nout\r\n");
    keys(&sub, &["M", "C"]);
    assert_eq!(
        logged(&sub),
        vec!["in 1..1: 0", "unknown: nil", "last none"]
    );
}

#[test]
fn commands_are_done_running_or_at_their_prompt() {
    let (_dir, sub) = built(COMMANDS);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07true\r\n\x1b]133;C\x07\x1b]133;D;0\x07\
          \x1b]133;A\x07$ \x1b]133;B\x07sleep 5\r\n\x1b]133;C\x07",
    );
    keys(&sub, &["C"]);
    assert_eq!(
        logged(&sub),
        vec!["done 0 1 0", "running 1 2 nil", "last running"]
    );

    more(&sub, b"\x1b]133;D;0\x07\x1b]133;A\x07$ ");
    keys(&sub, &["C"]);
    assert_eq!(
        logged(&sub),
        vec![
            "done 0 1 0",
            "done 1 2 0",
            "prompt 2 nil nil",
            "last prompt"
        ]
    );
}

/// `ssh` or `bash --norc` inside a shell that marks: the inner command's output started and no
/// `D` ever came, and the next prompt is the outer shell's. The inner command is not left
/// running.
#[test]
fn a_command_whose_done_never_came_is_unknown_once_a_prompt_follows() {
    let (_dir, sub) = built(COMMANDS);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07ssh host\r\n\x1b]133;C\x07remote$ ls\r\nx\r\n\
          remote$ exit\r\n\x1b]133;A\x07$ ",
    );
    keys(&sub, &["C"]);
    assert_eq!(
        logged(&sub),
        vec!["unknown 0 1 nil", "prompt 4 nil nil", "last prompt"]
    );
}

/// The first `D` a shell sends has no command before it, and one whose prompt fell off the
/// history has none either: it starts nothing.
#[test]
fn a_done_with_no_prompt_before_it_is_skipped() {
    let (_dir, sub) = built(COMMANDS);
    feed(&sub, b"\x1b]133;D;0\x07\x1b]133;A\x07$ ");
    keys(&sub, &["C"]);
    assert_eq!(logged(&sub), vec!["prompt 0 nil nil", "last prompt"]);
}

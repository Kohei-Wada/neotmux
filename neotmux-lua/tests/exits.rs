//! `plugin.exits`: how each command ended and how long it took, down each pane's left edge by
//! `C-b e`, out of the shell's OSC 133 marks.

mod support;

use neotmux_proto::{Color, DrawCmd, Epoch, Geometry, Rect};
use support::{Built, SIZE, built, feed, keys, one, pane};

const SHOWN: &str = "require('plugin.exits')\n\
    neotmux.on_key(function(key)\n\
      if key ~= 'E' then return false end\n\
      require('plugin.exits').toggle()\n\
      return true\n\
    end)\n";

const OK: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
const FAILED: Color = Color::Rgb(0xf3, 0x8b, 0xa8);

/// The host once it has taken the band off the pane: the outer rect over the whole surface and
/// the content nine columns in.
fn banded(sub: &Built) {
    let outer = Rect::new(0, 0, SIZE.cols, SIZE.rows);
    sub.begin_frame(
        one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: outer,
            panes: vec![(pane(), Rect::new(9, 0, SIZE.cols - 9, SIZE.rows))],
            outer: vec![(pane(), outer)],
            focus: Some(pane()),
            ..Default::default()
        },
    );
}

/// Every text the band drew that is not its rule, as (x, y, text, colour).
fn drawn(sub: &Built) -> Vec<(u16, u16, String, Color)> {
    sub.dispatch_tick(one());
    sub.frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { x, y, text, fg, .. } if x < 9 && text != "│" => {
                Some((x, y, text, fg))
            }
            DrawCmd::Rect { .. } => panic!("the band paints no background: {cmd:?}"),
            _ => None,
        })
        .collect()
}

fn rule(sub: &Built) -> usize {
    sub.dispatch_tick(one());
    sub.frame_output()
        .into_iter()
        .filter(|cmd| matches!(cmd, DrawCmd::Text { x: 8, text, .. } if text == "│"))
        .count()
}

fn more(sub: &Built, text: &[u8]) {
    sub.terminals().feed(pane(), text);
}

fn shown(sub: &Built) {
    keys(sub, &["E"]);
    banded(sub);
}

#[test]
fn a_command_that_exited_zero_is_ticked_on_the_line_it_was_typed_on() {
    let (_dir, sub) = built(SHOWN);
    feed(
        &sub,
        b"\r\n\x1b]133;A\x07$ \x1b]133;B\x07true\r\n\x1b]133;C\x07\x1b]133;D;0\x07\x1b]133;A\x07$ ",
    );
    shown(&sub);
    let texts = drawn(&sub);
    assert!(texts.contains(&(0, 1, "✔".to_string(), OK)), "{texts:?}");
    assert!(
        !texts.iter().any(|(_, y, _, _)| *y == 0 || *y == 2),
        "only the command's own line is marked: {texts:?}"
    );
}

#[test]
fn a_failed_command_shows_its_status() {
    let (_dir, sub) = built(SHOWN);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07false\r\n\x1b]133;C\x07\x1b]133;D;127\x07\x1b]133;A\x07$ ",
    );
    shown(&sub);
    let texts = drawn(&sub);
    assert!(
        texts.contains(&(0, 0, "✘".to_string(), FAILED)),
        "{texts:?}"
    );
    assert!(
        texts.contains(&(1, 0, "127".to_string(), FAILED)),
        "{texts:?}"
    );
}

#[test]
fn a_running_command_is_marked_as_running() {
    let (_dir, sub) = built(SHOWN);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07sleep 9\r\n\x1b]133;C\x07",
    );
    shown(&sub);
    let texts = drawn(&sub);
    assert!(
        texts
            .iter()
            .any(|(x, y, t, _)| (*x, *y) == (0, 0) && t == "⋯"),
        "{texts:?}"
    );
}

/// The time is when the plugin first saw the output start and the command end, so a command it
/// watched says how long it took, and one that had already finished when it looked says nothing.
#[test]
fn a_command_it_watched_says_how_long_it_took() {
    let (_dir, sub) = built(SHOWN);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07true\r\n\x1b]133;C\x07\x1b]133;D;0\x07\r\n",
    );
    sub.dispatch_tick(one());
    more(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07sleep 1\r\n\x1b]133;C\x07",
    );
    sub.dispatch_tick(one());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    more(&sub, b"\x1b]133;D;0\x07\r\n\x1b]133;A\x07$ ");
    sub.dispatch_tick(one());
    shown(&sub);
    let texts = drawn(&sub);
    let times: Vec<(u16, String)> = texts
        .iter()
        .filter(|(x, _, _, _)| *x == 5)
        .map(|(_, y, t, _)| (*y, t.clone()))
        .collect();
    assert_eq!(times, vec![(2, " 1s".to_string())], "{texts:?}");
}

#[test]
fn a_shell_that_sends_no_marks_gets_an_empty_band() {
    let (_dir, sub) = built(SHOWN);
    feed(&sub, b"$ ls\r\nout\r\n$ ");
    shown(&sub);
    assert_eq!(drawn(&sub), vec![]);
    assert_eq!(rule(&sub), usize::from(SIZE.rows));
}

#[test]
fn hiding_the_band_gives_the_columns_back() {
    let (_dir, sub) = built(SHOWN);
    assert_eq!(sub.pane_band(), 0);
    keys(&sub, &["E"]);
    assert_eq!(sub.pane_band(), 9);
    keys(&sub, &["E"]);
    assert_eq!(sub.pane_band(), 0);
}

/// The cursor and mode the host would publish for the pane every frame; the harness has no host.
fn publish(sub: &Built) {
    let tty = sub.terminals().screen_tty(pane()).expect("the pane");
    sub.publish_pane_tty(one(), std::collections::BTreeMap::from([(pane(), tty)]));
}

/// A full-screen program has no history, so the screen's top is the oldest line the pane holds
/// and every old command would land beside the program's rows. Nothing is drawn beside it.
#[test]
fn nothing_is_marked_beside_a_full_screen_program() {
    let (_dir, sub) = built(SHOWN);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07true\r\n\x1b]133;C\x07\x1b]133;D;0\x07\
          \x1b]133;A\x07$ \x1b]133;B\x07vim\r\n\x1b]133;C\x07\x1b[?1049h",
    );
    publish(&sub);
    shown(&sub);
    assert_eq!(drawn(&sub), vec![]);
}

/// A prompt of two lines puts its `input` mark on the second, which is the line the command
/// was typed on.
#[test]
fn a_two_line_prompt_is_marked_on_the_line_the_command_was_typed_on() {
    let (_dir, sub) = built(SHOWN);
    feed(
        &sub,
        b"\x1b]133;A\x07~/src main\r\n$ \x1b]133;B\x07true\r\n\x1b]133;C\x07\x1b]133;D;0\x07\
          \x1b]133;A\x07",
    );
    shown(&sub);
    let texts = drawn(&sub);
    assert!(texts.contains(&(0, 1, "✔".to_string(), OK)), "{texts:?}");
}

/// A `done` that carried no status says the command ended, not that it succeeded.
#[test]
fn a_done_with_no_status_is_not_shown_as_a_success() {
    let (_dir, sub) = built(SHOWN);
    feed(
        &sub,
        b"\x1b]133;A\x07$ \x1b]133;B\x07true\r\n\x1b]133;C\x07\x1b]133;D\x07\x1b]133;A\x07$ ",
    );
    shown(&sub);
    let texts = drawn(&sub);
    assert!(
        texts
            .iter()
            .any(|(x, y, t, _)| (*x, *y) == (0, 0) && t == "?"),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|(_, _, t, _)| t == "✔"), "{texts:?}");
}

/// Copy mode winds the pane back into its history with `ctx:view`; a command up there is marked
/// beside its own line, the way it would be if it were on the screen.
#[test]
fn a_command_in_the_history_is_marked_when_the_view_winds_back_to_it() {
    let (_dir, sub) = built(&format!(
        "local back = nil\n\
         neotmux.on_tick(function(ctx) if back then ctx:view(1, back) end end)\n\
         neotmux.on_key(function(key) if key ~= 'V' then return false end back = 6 return true end)\n\
         {SHOWN}"
    ));
    let mut bytes =
        b"\x1b]133;A\x07$ \x1b]133;B\x07true\r\n\x1b]133;C\x07\x1b]133;D;0\x07".to_vec();
    for i in 0..20 {
        bytes.extend(format!("x{i}\r\n").as_bytes());
    }
    bytes.extend(b"\x1b]133;A\x07$ ");
    feed(&sub, &bytes);
    shown(&sub);
    assert_eq!(drawn(&sub), vec![], "the command is in the history");

    keys(&sub, &["V"]);
    banded(&sub);
    let texts = drawn(&sub);
    assert!(texts.contains(&(0, 0, "✔".to_string(), OK)), "{texts:?}");
}

/// A command watched on the screen keeps its time once its lines have scrolled into the history,
/// so winding the view back to it shows how long it took, not a time counted from the moment the
/// view got there.
#[test]
fn a_command_keeps_its_time_after_scrolling_into_the_history() {
    let (_dir, sub) = built(&format!(
        "local back = nil\n\
         neotmux.on_tick(function(ctx) if back then ctx:view(1, back) end end)\n\
         neotmux.on_key(function(key) if key ~= 'V' then return false end back = 20 return true end)\n\
         {SHOWN}"
    ));
    feed(&sub, b"\x1b]133;A\x07$ \x1b]133;B\x07sleep 1\r\n");
    sub.dispatch_tick(one());
    more(&sub, b"\x1b]133;C\x07");
    sub.dispatch_tick(one());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let mut bytes = b"\x1b]133;D;0\x07\r\n".to_vec();
    for i in 0..20 {
        bytes.extend(format!("x{i}\r\n").as_bytes());
        more(&sub, &std::mem::take(&mut bytes));
        sub.dispatch_tick(one());
    }
    more(&sub, b"\x1b]133;A\x07$ ");
    sub.dispatch_tick(one());
    keys(&sub, &["V"]);
    shown(&sub);
    let texts = drawn(&sub);
    let times: Vec<String> = texts
        .iter()
        .filter(|(x, _, _, _)| *x == 5)
        .map(|(_, _, t, _)| t.clone())
        .collect();
    assert_eq!(times, vec![" 1s".to_string()], "{texts:?}");
}

/// The pane only in a window nobody is looking at, as the host publishes it then: in the
/// window's list, and not among the panes on the screen.
fn hidden(sub: &Built) {
    sub.begin_frame(
        one(),
        Epoch::new(SIZE, vec![], None),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            windows: vec![neotmux_proto::geom::Window {
                id: neotmux_proto::WindowId::from(1),
                panes: vec![pane()],
                name: None,
            }],
            window: Some(neotmux_proto::WindowId::from(2)),
            ..Default::default()
        },
    );
}

/// `clear` in a pane nobody is looking at writes over a command's marks, and the command that
/// follows does not inherit the old one's time.
#[test]
fn a_hidden_pane_forgets_the_marks_written_over() {
    let (_dir, sub) = built(SHOWN);
    feed(&sub, b"\x1b]133;A\x07$ \x1b]133;B\x07c1\r\n");
    sub.dispatch_tick(one());
    more(&sub, b"\x1b]133;C\x07");
    sub.dispatch_tick(one());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    hidden(&sub);
    more(
        &sub,
        b"\x1b]133;D;0\x07\x1b[H\x1b[2J\x1b]133;A\x07$ \x1b]133;B\x07c2\r\n",
    );
    sub.dispatch_tick(one());
    more(&sub, b"\x1b]133;C\x07\x1b]133;D;0\x07\r\n\x1b]133;A\x07$ ");
    sub.dispatch_tick(one());
    shown(&sub);
    let texts = drawn(&sub);
    let times: Vec<(u16, String)> = texts
        .iter()
        .filter(|(x, _, _, _)| *x == 5)
        .map(|(_, y, t, _)| (*y, t.clone()))
        .collect();
    assert_eq!(times, vec![(0, " 0s".to_string())], "{texts:?}");
}

/// The times and the exits fit side by side under the pane bands' cap, and neither is refused.
#[test]
fn the_times_and_the_exits_fit_side_by_side() {
    let (_dir, sub) = built(&format!(
        "{SHOWN}\n\
         require('plugin.stamps')\n\
         neotmux.on_key(function(key)\n\
           if key ~= 'T' then return false end\n\
           require('plugin.stamps').toggle()\n\
           return true\n\
         end)\n"
    ));
    keys(&sub, &["T", "E"]);
    assert_eq!(sub.pane_band(), 19);
    assert!(support::notified(&sub).is_empty());
}

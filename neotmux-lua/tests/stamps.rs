//! `plugin.stamps`: when each line of a pane arrived, shown down its left edge by `C-b t`.

mod support;

use neotmux_lua::CommandOutcome;
use neotmux_proto::{Color, DrawCmd, Epoch, Geometry, Rect};
use support::{Built, SIZE, built, feed, keys, notified, one, pane};

const SHOWN: &str = "require('plugin.stamps')\n\
    neotmux.on_key(function(key)\n\
      if key ~= 'T' then return false end\n\
      require('plugin.stamps').toggle()\n\
      return true\n\
    end)\n";

/// `SHOWN`, plus a command a test can run to read `M.shown` without reaching into the module.
const SHOWN_WITH_PROBE: &str = "require('plugin.stamps')\n\
    neotmux.on_key(function(key)\n\
      if key ~= 'T' then return false end\n\
      require('plugin.stamps').toggle()\n\
      return true\n\
    end)\n\
    neotmux.command.add('stamps-shown', function()\n\
      return { tostring(require('plugin.stamps').shown) }\n\
    end)\n";

/// What the host publishes once it has taken the band off the pane: the pane's outer rect over
/// the whole surface and its content ten columns in.
fn banded(sub: &Built) {
    let outer = Rect::new(0, 0, SIZE.cols, SIZE.rows);
    sub.begin_frame(
        one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: outer,
            panes: vec![(pane(), Rect::new(10, 0, SIZE.cols - 10, SIZE.rows))],
            outer: vec![(pane(), outer)],
            focus: Some(pane()),
            ..Default::default()
        },
    );
}

/// Each row's time, with the colour it is written in, and the rows the rule is drawn down.
fn gutter(sub: &Built) -> (Vec<(u16, String, Color)>, Vec<u16>) {
    sub.dispatch_tick(one());
    let mut times = Vec::new();
    let mut rule = Vec::new();
    for cmd in sub.frame_output() {
        match cmd {
            DrawCmd::Text {
                x: 0, y, text, fg, ..
            } if is_time(&text) => times.push((y, text, fg)),
            DrawCmd::Text { x: 9, y, text, .. } if text == "│" => rule.push(y),
            DrawCmd::Rect { .. } => panic!("the band paints no background: {cmd:?}"),
            _ => {}
        }
    }
    (times, rule)
}

fn is_time(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 8
        && b[2] == b':'
        && b[5] == b':'
        && text.chars().filter(|c| c.is_ascii_digit()).count() == 6
}

/// Fed, and the cursor told to the Lua layer where the terminal left it, as the host does
/// every frame: the harness has no host to publish it.
fn more(sub: &Built, text: &[u8]) {
    sub.terminals().feed(pane(), text);
    publish(sub);
}

fn publish(sub: &Built) {
    let tty = sub.terminals().screen_tty(pane()).expect("the pane");
    sub.publish_pane_tty(one(), std::collections::BTreeMap::from([(pane(), tty)]));
}

/// Three lines that arrive together are three stamped rows, and the time is written on the
/// first of them only — the other two are the same moment. Nothing is drawn until asked.
#[test]
fn lines_that_came_together_share_one_time_written_once() {
    let (_dir, sub) = built(SHOWN);
    feed(&sub, b"");
    publish(&sub);
    sub.dispatch_tick(one());
    more(&sub, b"one\r\ntwo\r\nthree\r\n");
    let (times, rule) = gutter(&sub);
    assert!(
        times.is_empty() && rule.is_empty(),
        "drawn before it was asked for"
    );

    keys(&sub, &["T"]);
    banded(&sub);
    let (times, rule) = gutter(&sub);
    assert_eq!(times.len(), 1, "{times:?}");
    assert_eq!(times[0].0, 0, "the time is on the first of them: {times:?}");
    assert_eq!(rule, (0..SIZE.rows).collect::<Vec<_>>());
}

/// A full-screen program's rows are not lines: nothing is stamped on the alternate screen.
#[test]
fn nothing_is_stamped_on_the_alternate_screen() {
    let (_dir, sub) = built(SHOWN);
    feed(&sub, b"\x1b[?1049h");
    publish(&sub);
    sub.dispatch_tick(one());
    more(&sub, b"top\r\nof\r\nhtop\r\n");
    keys(&sub, &["T"]);
    banded(&sub);
    let (times, _rule) = gutter(&sub);
    assert!(times.is_empty(), "{times:?}");
}

/// A line that came after the pane sat still for longer than `stall.slow` is painted so.
#[test]
fn a_line_after_a_wait_is_coloured_by_it() {
    let (_dir, sub) = built(&format!(
        "{SHOWN}require('plugin.stamps').stall = {{ slow = 1, stuck = 60 }}\n"
    ));
    feed(&sub, b"");
    publish(&sub);
    sub.dispatch_tick(one());
    more(&sub, b"before\r\n");
    sub.dispatch_tick(one());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    more(&sub, b"after\r\n");
    keys(&sub, &["T"]);
    banded(&sub);
    let (times, _rule) = gutter(&sub);
    let slow = Color::Rgb(0xf9, 0xe2, 0xaf);
    let on_row = |row: u16| times.iter().find(|(y, ..)| *y == row).map(|(_, _, fg)| *fg);
    assert_eq!(on_row(1), Some(slow), "{times:?}");
    assert_ne!(on_row(0), Some(slow), "{times:?}");
}

/// Hidden, the plugin holds no band, so the panes get their columns back.
#[test]
fn hiding_the_times_gives_the_columns_back() {
    let (_dir, sub) = built(SHOWN);
    assert_eq!(sub.pane_band(), 0);
    keys(&sub, &["T"]);
    assert_eq!(sub.pane_band(), 10);
    keys(&sub, &["T"]);
    assert_eq!(sub.pane_band(), 0);
}

/// Another source already holds the edge's whole cap: toggling on cannot take its 10 columns,
/// so it stays hidden, the total is unmoved, and the plugin says why.
#[test]
fn toggling_on_past_the_cap_stays_hidden_and_says_why() {
    let (_dir, sub) = built(&format!(
        "neotmux.reserve_pane('left', 15)\n{SHOWN_WITH_PROBE}"
    ));
    assert_eq!(sub.pane_band(), 15);
    keys(&sub, &["T"]);
    assert_eq!(sub.pane_band(), 15, "no double reservation");
    let CommandOutcome::Ran(lines) = sub.run_command(one(), "stamps-shown", &[]) else {
        panic!("stamps-shown did not run");
    };
    assert_eq!(lines, vec!["false".to_string()], "stays hidden");
    let notices = notified(&sub);
    assert!(
        notices.iter().any(|n| n.contains("plugin.stamps")),
        "{notices:?}"
    );
}

/// Copy mode winds the pane back with `ctx:view`; the times go with it, so each is still beside
/// its own line. Twenty lines that came together have one time, on the first of them, which is
/// in the history until the view is wound back to it.
#[test]
fn the_times_follow_a_view_wound_back_into_the_history() {
    let (_dir, sub) = built(&format!(
        "local back = nil\n\
         neotmux.on_tick(function(ctx) if back then ctx:view(1, back) end end)\n\
         neotmux.on_key(function(key) if key ~= 'V' then return false end back = 5 return true end)\n\
         {SHOWN}"
    ));
    feed(&sub, b"");
    publish(&sub);
    sub.dispatch_tick(one());
    let lines: String = (0..20).map(|i| format!("L{i}\r\n")).collect();
    more(&sub, lines.as_bytes());
    keys(&sub, &["T"]);
    banded(&sub);
    let (times, _) = gutter(&sub);
    assert!(
        times.is_empty(),
        "the first line is in the history: {times:?}"
    );

    keys(&sub, &["V"]);
    banded(&sub);
    let (times, _) = gutter(&sub);
    assert_eq!(
        times.iter().map(|(y, _, _)| *y).collect::<Vec<_>>(),
        vec![0]
    );
}

//! `plugin.stamps`: when each line of a pane arrived, shown down its left edge by `C-b t`.

mod support;

use neotmux_proto::DrawCmd;
use support::{Built, built, feed, keys, one, pane};

const SHOWN: &str = "require('plugin.stamps')\n\
    neotmux.on_key(function(key)\n\
      if key ~= 'T' then return false end\n\
      require('plugin.stamps').toggle()\n\
      return true\n\
    end)\n";

/// The `HH:MM:SS` drawn at each row of the gutter, and the rows painted with nothing on them.
fn gutter(sub: &Built) -> (Vec<(u16, String)>, Vec<u16>) {
    sub.dispatch_tick(one());
    let mut times = Vec::new();
    let mut painted = Vec::new();
    for cmd in sub.frame_output() {
        match cmd {
            DrawCmd::Text { x: 0, y, text, .. } if is_time(&text) => times.push((y, text)),
            DrawCmd::Rect { rect, .. } if rect.x == 0 => painted.push(rect.y),
            _ => {}
        }
    }
    (times, painted)
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
    let (times, painted) = gutter(&sub);
    assert!(
        times.is_empty() && painted.is_empty(),
        "drawn before it was asked for"
    );

    keys(&sub, &["T"]);
    let (times, painted) = gutter(&sub);
    assert_eq!(times.len(), 1, "{times:?}");
    assert_eq!(times[0].0, 0, "the time is on the first of them: {times:?}");
    assert_eq!(painted, vec![0, 1, 2]);
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
    let (times, painted) = gutter(&sub);
    assert!(
        times.is_empty() && painted.is_empty(),
        "{times:?} {painted:?}"
    );
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
    sub.dispatch_tick(one());
    let slow = neotmux_proto::Color::Rgb(0xf9, 0xe2, 0xaf);
    let rows: Vec<(u16, neotmux_proto::Color)> = sub
        .frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Rect { rect, bg, .. } if rect.x == 0 => Some((rect.y, bg)),
            _ => None,
        })
        .collect();
    assert!(
        rows.contains(&(1, slow)),
        "the line after the wait was not marked: {rows:?}"
    );
    assert!(!rows.contains(&(0, slow)), "{rows:?}");
}

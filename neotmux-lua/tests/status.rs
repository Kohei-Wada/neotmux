//! The shipped status bar, driven as the host drives it: the real `runtime/` layer, a frame,
//! and the draw commands it produced. What a PTY test can only see as characters on a screen
//! is a run of text with a foreground and a background here, which is where herdr's tab bar
//! is pinned: its widths, its colours and where it scrolls to.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{Color, DrawCmd, Epoch, Geometry, PaneId, Rect, Size, Window, WindowId};

mod support;

const COLS: u16 = 80;

const BAR: Color = rgb("#181825");
const BLUE: Color = rgb("#89b4fa");
const SURFACE0: Color = rgb("#313244");
const OVERLAY0: Color = rgb("#6c7086");
const OVERLAY1: Color = rgb("#7f849c");

const fn hex(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("colours in this test are lowercase #rrggbb"),
    }
}

/// `Color::Rgb` from the same `#rrggbb` the Lua writes, so the two can be compared by eye.
const fn rgb(s: &str) -> Color {
    let b = s.as_bytes();
    assert!(b.len() == 7 && b[0] == b'#');
    Color::Rgb(
        hex(b[1]) * 16 + hex(b[2]),
        hex(b[3]) * 16 + hex(b[4]),
        hex(b[5]) * 16 + hex(b[6]),
    )
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

/// The shipped layer, behind a user layer holding only the `init.lua` a user would write.
fn built(init: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

/// One frame of one pane in the window `current` among `windows`, each with the name it was
/// given if any, laid out the way the host lays one out: bands first, then the geometry the
/// panes were left with.
fn frame(
    sub: &LuaSubsystem,
    cols: u16,
    windows: &[(u32, Option<&str>)],
    current: u32,
    zoomed: bool,
) -> Vec<DrawCmd> {
    let surface = Size { rows: 24, cols };
    let reserved = sub.reserved();
    sub.lay_out_reservations(support::one(), surface, reserved);
    let content = Size {
        rows: surface.rows - reserved.rows(),
        cols,
    };
    let pane = PaneId::from(1);
    let ids = vec![pane];
    sub.begin_frame(
        support::one(),
        Epoch::new(content, ids.clone(), Some(pane)),
        Geometry {
            area: Rect::new(0, reserved.top, cols, content.rows),
            panes: vec![(pane, Rect::new(0, reserved.top, cols, content.rows))],
            focus: Some(pane),
            windows: windows
                .iter()
                .map(|(id, name)| Window {
                    id: WindowId::from(*id),
                    panes: if *id == current {
                        ids.clone()
                    } else {
                        Vec::new()
                    },
                    name: name.map(str::to_string),
                })
                .collect(),
            window: Some(WindowId::from(current)),
            zoomed: zoomed.then_some(pane),
        },
    );
    sub.dispatch_tick(support::one());
    sub.frame_output()
}

/// Every text run the frame drew, with its colours, in the order it drew them.
fn runs(cmds: &[DrawCmd]) -> Vec<(String, Color, Color)> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, fg, bg, .. } => Some((text.clone(), *fg, *bg)),
            _ => None,
        })
        .collect()
}

fn bold(cmds: &[DrawCmd], wanted: &str) -> bool {
    cmds.iter().any(
        |cmd| matches!(cmd, DrawCmd::Text { text, bold, .. } if text.trim() == wanted && *bold),
    )
}

fn unnamed(ids: &[u32]) -> Vec<(u32, Option<&'static str>)> {
    ids.iter().map(|id| (*id, None)).collect()
}

/// The row as it lands: every text at its column, later draws over earlier, trailing bar trimmed.
fn line(cmds: &[DrawCmd]) -> String {
    let mut cells = vec![' '; COLS as usize];
    for cmd in cmds {
        if let DrawCmd::Text { text, x, .. } = cmd {
            for (i, c) in text.chars().enumerate() {
                if let Some(cell) = cells.get_mut(*x as usize + i) {
                    *cell = c;
                }
            }
        }
    }
    cells.into_iter().collect::<String>().trim_end().to_string()
}

/// herdr's tab: the label centred in its width plus four, never under eight, one column of bar
/// between two; the one being shown in the accent, the rest on surface0.
#[test]
fn every_window_is_a_tab_and_the_current_one_is_in_the_accent() {
    let (_dir, sub) = built("");
    let cmds = frame(&sub, COLS, &unnamed(&[5, 9, 12]), 9, false);
    assert_eq!(
        runs(&cmds),
        vec![
            ("   1    ".to_string(), OVERLAY0, SURFACE0),
            ("   2    ".to_string(), BAR, BLUE),
            ("   3    ".to_string(), OVERLAY0, SURFACE0),
        ]
    );
}

/// The band is painted before anything lands in it, so the gaps between tabs are the bar.
#[test]
fn the_whole_row_is_painted_in_the_bars_colour() {
    let (_dir, sub) = built("");
    let cmds = frame(&sub, COLS, &unnamed(&[1]), 1, false);
    let painted = cmds.iter().find_map(|cmd| match cmd {
        DrawCmd::Rect { rect, bg, .. } => Some((*rect, *bg)),
        _ => None,
    });
    assert_eq!(painted, Some((Rect::new(0, 0, COLS, 1), BAR)));
}

/// A window you named shows its name, brighter than a number, and bold while it is shown.
#[test]
fn a_named_window_is_its_name() {
    let (_dir, sub) = built("");
    let cmds = frame(
        &sub,
        COLS,
        &[(1, Some("editor")), (2, Some("logs"))],
        1,
        false,
    );
    assert_eq!(
        runs(&cmds),
        vec![
            ("  editor  ".to_string(), BAR, BLUE),
            ("  logs  ".to_string(), OVERLAY1, SURFACE0),
        ]
    );
    assert!(bold(&cmds, "editor"));
    assert!(!bold(&cmds, "logs"));
}

#[test]
fn a_zoomed_window_says_so() {
    let (_dir, sub) = built("");
    let text = line(&frame(&sub, COLS, &unnamed(&[1, 2]), 2, true));
    assert_eq!(text, "   1       2 Z");
}

/// Too many to fit: the strip starts where the current tab sits nearest the middle, and an
/// ellipsis at either end says there is more that way.
#[test]
fn a_strip_too_long_for_the_row_scrolls_to_the_current_tab() {
    let (_dir, sub) = built("");
    let ids: Vec<u32> = (1..=15).collect();
    let text = line(&frame(&sub, COLS, &unnamed(&ids), 10, false));
    assert!(text.starts_with('…'), "{text}");
    assert!(text.contains("   10   "), "{text}");
    assert!(!text.contains("   1    "), "{text}");

    let first = line(&frame(&sub, COLS, &unnamed(&ids), 1, false));
    assert!(first.starts_with("   1    "), "{first}");
    assert!(first.ends_with('…'), "{first}");
}

/// The last tabs are shown whole rather than the strip scrolled past them.
#[test]
fn the_strip_never_scrolls_past_its_last_tab() {
    let (_dir, sub) = built("");
    let ids: Vec<u32> = (1..=15).collect();
    let text = line(&frame(&sub, COLS, &unnamed(&ids), 15, false));
    assert!(text.ends_with("   15"), "{text}");
}

/// herdr's `tab_bar_right`: empty unless given, each entry separated, the accented one in the
/// accent, and flush right.
#[test]
fn what_is_given_for_the_right_is_drawn_flush_right() {
    let (_dir, sub) = built(
        "local s = require('plugin.tmux.status')\n\
         s.right = {\n\
           function() return 'prod', true end,\n\
           function() return nil end,\n\
           function() return '12:00' end,\n\
         }\n\
         s.separator = ' · '",
    );
    let cmds = frame(&sub, COLS, &unnamed(&[1]), 1, false);
    assert_eq!(
        runs(&cmds)[..3],
        [
            ("prod".to_string(), BAR, BLUE),
            (" · ".to_string(), OVERLAY0, BAR),
            ("12:00".to_string(), OVERLAY1, BAR),
        ]
    );
    let at = cmds.iter().find_map(|cmd| match cmd {
        DrawCmd::Text { text, x, .. } if text == "prod" => Some(*x),
        _ => None,
    });
    assert_eq!(at, Some(COLS - "prod · 12:00".chars().count() as u16));
}

/// And a right side that would leave the strip too narrow is left off rather than squeezed.
#[test]
fn a_right_side_with_no_room_is_left_off() {
    let (_dir, sub) = built(
        "require('plugin.tmux.status').right = { function() return string.rep('x', 70) end }",
    );
    let text = line(&frame(&sub, COLS, &unnamed(&[1]), 1, false));
    assert!(!text.contains('x'), "{text}");
}

/// A colour is a field, and the field is read on the tick.
#[test]
fn a_colour_field_reaches_the_screen() {
    let (_dir, sub) = built("require('plugin.tmux.status').colors.accent = '#ff0000'");
    let cmds = frame(&sub, COLS, &unnamed(&[1]), 1, false);
    assert_eq!(runs(&cmds)[0].2, rgb("#ff0000"));
}

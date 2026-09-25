//! The shipped status bar, driven as the host drives it: the real `runtime/` layer, a frame,
//! and the draw commands it produced. What a PTY test can only see as characters on a screen
//! is a run of text with a foreground and a background here, which is where the colours and
//! the glyphs are pinned.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{
    Color, DrawCmd, Epoch, Geometry, PaneId, PaneInfo, Rect, Size, Window, WindowId,
};

mod support;

const COLS: u16 = 80;
const SURFACE: Size = Size {
    rows: 24,
    cols: COLS,
};

const BAR: Color = rgb("#181825");
const MAUVE: Color = rgb("#cba6f7");
const SURFACE0: Color = rgb("#313244");
const SURFACE1: Color = rgb("#45475a");
const TEXT: Color = rgb("#cdd6f4");
const CRUST: Color = rgb("#11111b");
const MAROON: Color = rgb("#eba0ac");
const GREEN: Color = rgb("#a6e3a1");

const CAP_LEFT: &str = "\u{e0b6}";
const CAP_RIGHT: &str = "\u{e0b4}";
const COMMAND_ICON: &str = "\u{f1ae}";
const DIRECTORY_ICON: &str = "\u{f07b}";

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

/// The shipped layer with nothing configured, under the `$HOME` the host published.
fn built_with_home(home: &str) -> LuaSubsystem {
    let layers = Layers::from_paths(vec![shipped_layer()]).with_home(PathBuf::from(home));
    let sub = LuaSubsystem::build(layers, support::panes()).unwrap();
    assert!(sub.run_init().is_empty());
    sub
}

fn info(command: &str, cwd: &str) -> PaneInfo {
    PaneInfo {
        command: Some(command.to_string()),
        cwd: Some(cwd.to_string()),
    }
}

/// One frame with the given panes, the given focus and the given windows, laid out the way
/// the host lays one out: bands first, then the geometry the panes were left with.
fn frame(
    sub: &LuaSubsystem,
    panes: &[(u32, PaneInfo)],
    focus: u32,
    windows: &[u32],
    current: u32,
) -> Vec<DrawCmd> {
    let reserved = sub.reserved();
    sub.lay_out_reservations(support::one(), SURFACE, reserved);
    let content = Size {
        rows: SURFACE.rows - reserved.rows(),
        cols: COLS,
    };
    sub.publish_pane_info(
        support::one(),
        panes
            .iter()
            .map(|(id, info)| (PaneId::from(*id), info.clone()))
            .collect::<BTreeMap<_, _>>(),
    );
    let ids: Vec<PaneId> = panes.iter().map(|(id, _)| PaneId::from(*id)).collect();
    sub.begin_frame(
        support::one(),
        Epoch::new(content, ids.clone(), Some(PaneId::from(focus))),
        Geometry {
            area: Rect::new(0, reserved.top, COLS, content.rows),
            panes: ids
                .iter()
                .map(|id| (*id, Rect::new(0, reserved.top, COLS, content.rows)))
                .collect(),
            focus: Some(PaneId::from(focus)),
            windows: windows
                .iter()
                .copied()
                .map(|id| Window {
                    id: WindowId::from(id),
                    // A status bar asks for the window ids; only the shown window's panes
                    // are ever read out of this fixture.
                    panes: if id == current {
                        ids.clone()
                    } else {
                        Vec::new()
                    },
                    name: None,
                })
                .collect(),
            window: Some(WindowId::from(current)),
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

fn line(cmds: &[DrawCmd]) -> String {
    runs(cmds).into_iter().map(|(text, _, _)| text).collect()
}

/// The bar the capture was taken from, run for run: the current window capped in mauve with
/// what its focused pane is running beside it, then that command and that directory as the
/// two right-hand modules.
#[test]
fn the_bar_is_composed_of_the_runs_it_was_drawn_from() {
    let (_dir, sub) = built("require('plugin.status').home = '/home/kohei'");
    let cmds = frame(&sub, &[(1, info("bash", "/home/kohei/life"))], 1, &[1], 1);

    assert_eq!(
        runs(&cmds),
        vec![
            (CAP_LEFT.to_string(), MAUVE, BAR),
            ("1.1 ".to_string(), BAR, MAUVE),
            (" bash".to_string(), TEXT, SURFACE1),
            (CAP_RIGHT.to_string(), SURFACE1, BAR),
            (" ".to_string(), BAR, BAR),
            (CAP_LEFT.to_string(), MAROON, BAR),
            (format!("{COMMAND_ICON} "), CRUST, MAROON),
            (" bash ".to_string(), TEXT, SURFACE0),
            (CAP_LEFT.to_string(), GREEN, BAR),
            (format!("{DIRECTORY_ICON} "), CRUST, GREEN),
            (" ~/life ".to_string(), TEXT, SURFACE0),
        ]
    );
}

/// The band is painted before anything lands in it, so the row is the bar's colour from end
/// to end rather than whatever the terminal's default happens to be.
#[test]
fn the_whole_row_is_painted_in_the_bars_colour() {
    let (_dir, sub) = built("");
    let cmds = frame(&sub, &[(1, info("bash", "/tmp"))], 1, &[1], 1);
    let painted = cmds.iter().find_map(|cmd| match cmd {
        DrawCmd::Rect { rect, bg, .. } => Some((*rect, *bg)),
        _ => None,
    });
    assert_eq!(painted, Some((Rect::new(0, 0, COLS, 1), BAR)));
}

/// The requirement the user stated first: what the bar *says* follows the focus, not merely
/// that the focus moved.
#[test]
fn the_bar_follows_the_focused_pane() {
    let (_dir, sub) = built("require('plugin.status').home = '/home/kohei'");
    let panes = [
        (1, info("bash", "/home/kohei/life")),
        (2, info("nvim", "/etc")),
    ];

    let first = line(&frame(&sub, &panes, 1, &[1], 1));
    let second = line(&frame(&sub, &panes, 2, &[1], 1));

    assert_ne!(first, second);
    assert!(
        first.contains("bash") && first.contains("~/life"),
        "{first}"
    );
    assert!(
        second.contains("nvim") && second.contains("/etc"),
        "{second}"
    );
    assert!(!second.contains("bash"), "{second}");
}

/// Every window is listed, in `window.list()` order, and only the one being shown is capped
/// in the current colour.
///
/// The ids here are 5, 9 and 12 with pane 4 focused — none of them a position — because what
/// a block is numbered with is its place in the list and not the id, which is allocated once
/// and never reused. The window being shown carries the focused pane's position after a dot.
#[test]
fn every_window_is_listed_and_the_current_one_is_capped_in_mauve() {
    let (_dir, sub) = built("");
    let cmds = frame(&sub, &[(4, info("bash", "/tmp"))], 4, &[5, 9, 12], 9);
    let indices: Vec<(String, Color)> = runs(&cmds)
        .into_iter()
        .filter(|(text, _, _)| matches!(text.as_str(), "1 " | "2.1 " | "3 "))
        .map(|(text, _, bg)| (text, bg))
        .collect();

    assert_eq!(
        indices,
        vec![
            ("1 ".to_string(), SURFACE0),
            ("2.1 ".to_string(), MAUVE),
            ("3 ".to_string(), SURFACE0),
        ]
    );
    let text = line(&cmds);
    assert!(
        text.starts_with(&format!("{CAP_LEFT}1 {CAP_RIGHT} ")),
        "{text}"
    );
    assert!(
        text.contains(&format!("{CAP_LEFT}2.1  bash{CAP_RIGHT} ")),
        "{text}"
    );
    assert!(
        text.contains(&format!("{CAP_LEFT}3 {CAP_RIGHT} ")),
        "{text}"
    );
}

/// A cap is the half-circle that closes a block: drawn in the block's own colour on the bar
/// behind it. The other way round it reads as a notch bitten out of the block.
#[test]
fn every_cap_is_its_blocks_colour_on_the_bar() {
    let (_dir, sub) = built("");
    let cmds = frame(&sub, &[(1, info("bash", "/tmp"))], 1, &[1, 2], 1);
    let caps: Vec<(String, Color, Color)> = runs(&cmds)
        .into_iter()
        .filter(|(text, _, _)| text == CAP_LEFT || text == CAP_RIGHT)
        .collect();

    assert_eq!(
        caps,
        vec![
            (CAP_LEFT.to_string(), MAUVE, BAR),
            (CAP_RIGHT.to_string(), SURFACE1, BAR),
            (CAP_LEFT.to_string(), SURFACE0, BAR),
            (CAP_RIGHT.to_string(), SURFACE0, BAR),
            (CAP_LEFT.to_string(), MAROON, BAR),
            (CAP_LEFT.to_string(), GREEN, BAR),
        ]
    );
}

/// A colour is a field, and the field is read on the tick: changing one changes what reaches
/// the screen and nothing else.
#[test]
fn a_colour_field_reaches_the_screen() {
    let (_dir, sub) = built("require('plugin.status').colors.command = '#ff0000'");
    let cmds = frame(&sub, &[(1, info("bash", "/tmp"))], 1, &[1], 1);
    let icon = runs(&cmds)
        .into_iter()
        .find(|(text, _, _)| text.starts_with(COMMAND_ICON))
        .expect("the command module");
    assert_eq!(icon.2, rgb("#ff0000"));
}

/// And so is a glyph: the one line someone without a Nerd Font writes.
#[test]
fn emptying_the_caps_takes_the_glyphs_off_the_bar() {
    let (_dir, sub) = built(
        "local s = require('plugin.status')\n\
         s.caps = { left = '', right = '' }\n\
         s.icons = { command = '', directory = '' }",
    );
    let text = line(&frame(&sub, &[(1, info("bash", "/tmp"))], 1, &[1], 1));
    assert_eq!(text, "1.1  bash   bash   /tmp ");
}

/// `$HOME` is written the way a prompt writes it.
#[test]
fn the_home_directory_is_written_as_a_tilde() {
    let (_dir, sub) = built("require('plugin.status').home = '/home/kohei'");
    let text = line(&frame(
        &sub,
        &[(1, info("bash", "/home/kohei"))],
        1,
        &[1],
        1,
    ));
    assert!(text.ends_with(" ~ "), "{text}");
}

/// A directory too long for what is left of the row loses its leading components, keeping
/// the end — the part that says where you are.
#[test]
fn a_directory_that_does_not_fit_keeps_its_tail() {
    let (_dir, sub) = built("require('plugin.status').home = '/home/kohei'");
    let deep = format!("/home/kohei/{}leaf", "middle/".repeat(12));
    let text = line(&frame(&sub, &[(1, info("bash", &deep))], 1, &[1], 1));
    assert!(
        text.contains(" .../middle/") && text.ends_with("/leaf "),
        "{text}"
    );
    assert!(!text.contains("~/middle"), "{text}");
    assert!(
        text.chars().count() <= COLS as usize,
        "the bar is {} columns wide: {text}",
        text.chars().count()
    );
}

/// And when not even its last component fits, the module goes rather than the bar being
/// drawn over itself.
#[test]
fn a_directory_with_no_room_at_all_is_dropped() {
    let (_dir, sub) = built("require('plugin.status').home = '/home/kohei'");
    let long = format!("/home/kohei/{}", "x".repeat(200));
    let text = line(&frame(&sub, &[(1, info("bash", &long))], 1, &[1], 1));
    assert!(!text.contains('x'), "{text}");
    assert!(text.contains("bash"), "{text}");
}

/// And the `~` is there with nothing configured at all: the host publishes `$HOME` and the
/// bar reads it. A directory outside it stays absolute.
#[test]
fn the_published_home_is_abbreviated_and_nothing_else_is() {
    let sub = built_with_home("/home/someone");

    let inside = line(&frame(
        &sub,
        &[(1, info("bash", "/home/someone/work"))],
        1,
        &[1],
        1,
    ));
    assert!(inside.ends_with(" ~/work "), "{inside}");

    let outside = line(&frame(&sub, &[(1, info("bash", "/srv/work"))], 1, &[1], 1));
    assert!(outside.ends_with(" /srv/work "), "{outside}");
}

/// With no `HOME` in the host's environment there is nothing to publish, so the bar writes
/// the path out in full rather than raising on a nil.
#[test]
fn with_no_home_published_the_path_stays_absolute() {
    let sub =
        LuaSubsystem::build(Layers::from_paths(vec![shipped_layer()]), support::panes()).unwrap();
    assert!(sub.run_init().is_empty());
    let text = line(&frame(
        &sub,
        &[(1, info("bash", "/home/someone/work"))],
        1,
        &[1],
        1,
    ));
    assert!(text.ends_with(" /home/someone/work "), "{text}");
}

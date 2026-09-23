//! The shipped copy mode, driven without a terminal: keys in, draw commands out. What a PTY
//! test can only see as pixels — a tint rather than a rect, a line that is not on the screen,
//! how many commands a scrolled frame costs — is a value here.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{Color, DrawCmd, Epoch, Event, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

/// Six rows, so ten printed lines leave four in the history.
const SIZE: Size = Size { rows: 6, cols: 20 };
const SELECTION: Color = Color::Rgb(0x45, 0x47, 0x5a);

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

fn pane() -> PaneId {
    PaneId::from(1)
}

/// The shipped runtime with the prefix and copy mode on and everything else off, so every
/// draw command in a frame is copy mode's.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    built_named(extra, std::rc::Rc::new(neotmux_lua::clipboard::Absent))
}

/// A clipboard that always accepts the write. For the tests that check what the mode says
/// when the terminal's clipboard is there to offer to, which `Absent` can never let happen.
struct WorkingClipboard;

impl neotmux_lua::clipboard::Clipboard for WorkingClipboard {
    fn set(&self, _text: &str) -> std::io::Result<()> {
        Ok(())
    }
}

fn built_named(
    extra: &str,
    clipboard: std::rc::Rc<dyn neotmux_lua::clipboard::Clipboard>,
) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             defaults.status = false\n\
             defaults.borders = false\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build_with(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        clipboard,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            size: SIZE,
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
    (dir, Built::new(sub, terminals))
}

/// Ten lines into a six-row pane: `one` through `four` are history, `five` through `ten` are
/// the visible screen.
fn scrolled(sub: &Built) {
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(
        pane(),
        b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\neight\r\nnine\r\nten",
    );
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        sub.dispatch_key(support::one(), spec);
    }
}

fn enter(sub: &LuaSubsystem) {
    keys(sub, &["C-b", "["]);
}

fn frame(sub: &LuaSubsystem) -> Vec<DrawCmd> {
    sub.dispatch_tick(support::one());
    sub.frame_output()
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn texts(cmds: &[DrawCmd]) -> Vec<String> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn tints(cmds: &[DrawCmd]) -> Vec<(Rect, Option<Color>)> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Tint { rect, bg, .. } => Some((*rect, *bg)),
            _ => None,
        })
        .collect()
}

/// The frame a mode that is not on draws is no frame at all: the tick returns before it has
/// asked the pane anything.
#[test]
fn nothing_is_drawn_until_the_mode_is_entered() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    assert!(frame(&sub).is_empty());
}

/// Entering puts the view exactly where the screen already is, so the pane's own grid is
/// still what shows it: the whole frame is the one cell the cursor is on.
#[test]
fn entering_costs_one_cell_and_leaves_the_pane_alone() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    let cmds = frame(&sub);
    assert_eq!(tints(&cmds).len(), 1, "{cmds:?}");
    assert_eq!(tints(&cmds)[0].0, Rect::new(0, 0, 1, 1));
    assert!(texts(&cmds).is_empty(), "the pane was redrawn: {cmds:?}");
}

/// The cursor is where the keys put it, and it is a cell you can see.
#[test]
fn the_cursor_moves_and_is_drawn_where_it_landed() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["l", "l", "l", "j"]);
    assert_eq!(tints(&frame(&sub))[0].0, Rect::new(3, 1, 1, 1));
}

/// Scrolling above the top of the screen is the point of the mode. `four` is the last line of
/// the history and is on no row of the visible screen, so a frame carrying it as text is a
/// frame built out of `pane.scrollback` and not out of `pane.content`.
#[test]
fn scrolling_above_the_screen_draws_lines_the_pane_no_longer_shows() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    let screen = sub
        .terminals()
        .content(pane(), None, neotmux_proto::read::Want::default())
        .unwrap()
        .lines;
    assert!(!screen.contains(&"four".to_string()), "{screen:?}");

    enter(&sub);
    keys(&sub, &["k"]);
    let cmds = frame(&sub);
    assert!(
        texts(&cmds).contains(&"four".to_string()),
        "the history was never read: {cmds:?}"
    );
    assert!(
        texts(&cmds).contains(&"nine".to_string()),
        "the view lost the rows it kept: {cmds:?}"
    );
}

/// `g` walks to the oldest line the pane still holds, which is the top of the history.
#[test]
fn g_lands_on_the_oldest_line_and_capital_g_on_the_newest() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["g"]);
    assert!(texts(&frame(&sub)).contains(&"one".to_string()));

    keys(&sub, &["G"]);
    let cmds = frame(&sub);
    assert!(
        texts(&cmds).is_empty(),
        "the bottom of the buffer is the screen and needs no redraw: {cmds:?}"
    );
    assert_eq!(tints(&cmds)[0].0, Rect::new(0, 5, 1, 1));
}

/// The selection is a tint, so the text under it is still there. A rect would be the same
/// rectangle with the pane's own characters gone, and this is the assertion that tells the
/// two apart without a screen: not one cell of the pane is painted over.
#[test]
fn the_selection_is_tinted_over_the_pane_and_never_painted_over_it() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &[" ", "j", "l", "l"]);
    let cmds = frame(&sub);

    assert!(
        !cmds.iter().any(|cmd| matches!(cmd, DrawCmd::Rect { .. })),
        "the selection erased the pane: {cmds:?}"
    );
    let selected: Vec<Rect> = tints(&cmds)
        .into_iter()
        .filter(|(_, bg)| *bg == Some(SELECTION))
        .map(|(rect, _)| rect)
        .collect();
    // From (0,0) to (2,1): the first row runs to the pane's right edge, the second stops at
    // the cursor.
    assert_eq!(
        selected,
        vec![Rect::new(0, 0, SIZE.cols, 1), Rect::new(0, 1, 3, 1)]
    );
}

/// A selection dragged upwards covers exactly the cells one dragged down over the same two
/// points does. The region is between two points; which of them the hand started on is not
/// one of its properties.
#[test]
fn a_selection_reads_the_same_way_round_whichever_way_it_was_dragged() {
    let spans = |drag: &[&str]| {
        let (_dir, sub) = built("");
        scrolled(&sub);
        enter(&sub);
        keys(&sub, drag);
        let out: Vec<Rect> = tints(&frame(&sub))
            .into_iter()
            .filter(|(_, bg)| *bg == Some(SELECTION))
            .map(|(rect, _)| rect)
            .collect();
        out
    };
    // (1, 0) to (1, 2), anchored at each end in turn.
    let down = spans(&["l", " ", "j", "j"]);
    let up = spans(&["j", "j", "l", " ", "k", "k"]);
    assert_eq!(
        down,
        vec![
            Rect::new(1, 0, SIZE.cols - 1, 1),
            Rect::new(0, 1, SIZE.cols, 1),
            Rect::new(0, 2, 2, 1),
        ]
    );
    assert_eq!(down, up);
}

/// A prefix command that says what is in the register, so a yank can be read back through
/// the same door a user has.
const REPORT: &str = "require('plugin.prefix').commands['R'] = function()\n\
                        neotmux.log.info('register=[' .. require('plugin.copy').register .. ']')\n\
                      end\n";

/// Yank takes the text of the region — across the boundary between the history and the
/// screen, because the mode does not have two buffers.
#[test]
fn yanking_puts_the_selected_text_in_the_register() {
    let (_dir, sub) = built_named(REPORT, std::rc::Rc::new(WorkingClipboard));
    scrolled(&sub);
    enter(&sub);
    // To the top of the history, select `one` and the two lines under it.
    keys(&sub, &["g", " ", "j", "j", "l", "l", "l", "l", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "register=[one\ntwo\nthree]"),
        "{said:?}"
    );
    assert!(
        said.iter().any(|line| line
            == "yanked 3 lines into plugin.copy.register, \
                and offered them to the terminal's clipboard"),
        "{said:?}"
    );
}

/// A yank reaching over the boundary is one region, not two: `four` is the last line of the
/// history and `five` the first of the screen.
#[test]
fn a_yank_reads_across_the_boundary_between_the_history_and_the_screen() {
    let (_dir, sub) = built(REPORT);
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["g", "j", "j", "j", " ", "j", "l", "l", "l", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "register=[four\nfive]"),
        "{said:?}"
    );
}

/// Every key in the mode is the mode's, and the first key after it is the shell's again.
#[test]
fn the_mode_swallows_its_keys_and_gives_them_back_on_the_way_out() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough
    );
    enter(&sub);
    assert_eq!(sub.dispatch_key(support::one(), "j"), KeyOutcome::Consumed);
    assert_eq!(sub.dispatch_key(support::one(), "z"), KeyOutcome::Consumed);
    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough
    );
    assert!(frame(&sub).is_empty(), "the mode kept drawing after Esc");
}

/// `q` is the other way out, and it leaves nothing behind either.
#[test]
fn q_leaves_the_mode() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    assert_eq!(sub.dispatch_key(support::one(), "q"), KeyOutcome::Consumed);
    assert_eq!(
        sub.dispatch_key(support::one(), "q"),
        KeyOutcome::Fallthrough
    );
}

/// What a scrolled frame costs, stated rather than guessed: one erase and one text command
/// per non-blank row of the pane, and nothing else while there is no selection. A six-row
/// pane showing six lines is seven commands.
#[test]
fn a_scrolled_frame_costs_one_erase_and_one_text_per_row() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["g"]);
    let cmds = frame(&sub);
    assert_eq!(
        cmds.iter()
            .filter(|cmd| matches!(cmd, DrawCmd::Rect { .. }))
            .count(),
        1,
        "{cmds:?}"
    );
    assert_eq!(texts(&cmds).len(), usize::from(SIZE.rows), "{cmds:?}");
    assert_eq!(tints(&cmds).len(), 1, "{cmds:?}");
}

/// The buffer is taken once, on the way in. A pane that keeps printing afterwards does not
/// change a line the cursor is already over — which is the whole reason for a snapshot rather
/// than a per-frame read, and the reason the text a yank takes is the text that was selected.
#[test]
fn what_the_pane_prints_after_the_mode_opens_is_not_in_the_buffer() {
    let (_dir, sub) = built(REPORT);
    scrolled(&sub);
    enter(&sub);
    sub.terminals().feed(pane(), b"\r\nELEVEN\r\nTWELVE");

    let cmds = frame(&sub);
    let drawn = texts(&cmds);
    assert!(
        drawn.contains(&"ten".to_string()),
        "the snapshot was not what got drawn: {drawn:?}"
    );
    assert!(
        !drawn.contains(&"ELEVEN".to_string()),
        "the mode redrew itself out of the live pane: {drawn:?}"
    );

    keys(&sub, &["G", " ", "l", "l", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "register=[ten]"),
        "the yank took what the pane says now rather than what was selected: {said:?}"
    );
}

/// A pane printing under the mode is also what takes the cheap frame away: the pane's own
/// grid no longer shows the snapshot's bottom, so the mode has to draw it.
#[test]
fn a_pane_that_prints_under_the_mode_costs_the_redraw_it_did_not_cost_before() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    assert!(texts(&frame(&sub)).is_empty(), "the quiet frame drew rows");
    sub.terminals().feed(pane(), b"\r\nELEVEN");
    assert!(
        !texts(&frame(&sub)).is_empty(),
        "the mode kept showing a grid that had moved under it"
    );
}

/// The selection is two absolute line numbers, so scrolling the view past one of its ends
/// does not touch it: drag the anchor off the top of the screen and the yank still takes
/// every line between the two points. Held in screen coordinates it would have evaporated.
#[test]
fn a_selection_survives_its_anchor_scrolling_off_the_view() {
    let (_dir, sub) = built(REPORT);
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["g", " ", "G", "l", "l", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line
                == "register=[one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten]"),
        "{said:?}"
    );
}

/// An end of the selection above the view is clamped to the edge rather than lost, so the
/// part of it that is on screen is still drawn.
#[test]
fn an_end_above_the_view_is_clamped_and_the_rest_still_shows() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    // Anchor on the oldest line, then drag to the bottom: the anchor scrolls off the top.
    keys(&sub, &["g", " ", "G"]);
    let selected: Vec<Rect> = tints(&frame(&sub))
        .into_iter()
        .filter(|(_, bg)| *bg == Some(SELECTION))
        .map(|(rect, _)| rect)
        .collect();
    assert_eq!(selected.len(), usize::from(SIZE.rows), "{selected:?}");
    assert_eq!(
        selected[0],
        Rect::new(0, 0, SIZE.cols, 1),
        "the clamped end did not start at the left edge"
    );
}

/// The keys are a table of command names, so one assignment rebinds one of them.
#[test]
fn a_key_can_be_rebound_from_a_user_config() {
    let (_dir, sub) = built("require('plugin.copy').keys['v'] = 'begin-selection'\n");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["v", "l"]);
    let selected = tints(&frame(&sub))
        .into_iter()
        .filter(|(_, bg)| *bg == Some(SELECTION))
        .count();
    assert_eq!(selected, 1, "the rebound key did not start a selection");
}

/// The mode belongs to the pane it was entered on: a pane that closes under it takes it with
/// it rather than leaving a cursor drawn over whatever moved in.
#[test]
fn a_pane_closing_under_the_mode_leaves_it() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    sub.dispatch_event(support::one(), &Event::PaneClosed { pane: pane() });
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough
    );
}

/// One line in one file: the default goes off the way the other five do.
#[test]
fn a_user_config_can_switch_copy_mode_off() {
    let (_dir, sub) = built("defaults.copy = false\n");
    scrolled(&sub);
    enter(&sub);
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough
    );
    assert!(frame(&sub).is_empty());
}

/// `clipboard.set` returning true says the escape reached the terminal, not that the terminal
/// took it — OSC 52 has no reply, and one that ignores it is byte-identical to one that
/// accepted it. So a yank says the register plainly and the clipboard as an offer. Claiming
/// the clipboard holds the text is the false-success other multiplexers ship.
#[test]
fn a_yank_claims_the_register_and_only_offers_the_clipboard() {
    let (_dir, sub) = built_named("", std::rc::Rc::new(WorkingClipboard));
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &[" ", "l", "y"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line
            == "yanked 1 line into plugin.copy.register, \
                and offered them to the terminal's clipboard"),
        "{said:?}"
    );
    assert!(
        !said
            .iter()
            .any(|line| line.contains("copied") && line.contains("clipboard")),
        "the yank claimed a clipboard: {said:?}"
    );
}

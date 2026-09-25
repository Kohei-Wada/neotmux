//! The shipped copy mode, driven without a terminal: keys in, draw commands out. What a PTY
//! test can only see as pixels — a tint rather than a rect, a line that is not on the screen,
//! how many commands a scrolled frame costs — is a value here.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{Color, DrawCmd, Epoch, Event, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

/// Six rows, so ten printed lines leave four in the history.
const SIZE: Size = Size { rows: 6, cols: 20 };
const SELECTION: Color = Color::Rgb(0x45, 0x47, 0x5a);

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

fn pane() -> PaneId {
    PaneId::from(1)
}

/// The shipped runtime with the prefix and copy mode on and everything else off, so every
/// draw command in a frame is copy mode's.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    built_named(extra, std::rc::Rc::new(neotmux_lua::host::Absent))
}

/// A clipboard that always accepts the write. For the tests that check what the mode says
/// when the terminal's clipboard is there to offer to, which `Absent` can never let happen.
struct WorkingClipboard;

impl neotmux_lua::host::Clipboard for WorkingClipboard {
    fn set(&self, _text: &str) -> std::io::Result<()> {
        Ok(())
    }
}

fn built_named(
    extra: &str,
    clipboard: std::rc::Rc<dyn neotmux_lua::host::Clipboard>,
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
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
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

/// How far back into its own history the frame is showing the pane. The mode says it with
/// `ctx:view` and the host snapshots the pane there, so this — and not a list of drawn rows —
/// is what "the view has scrolled" now looks like from outside.
fn view(sub: &LuaSubsystem) -> usize {
    sub.dispatch_tick(support::one());
    sub.frame_views().get(&pane()).copied().unwrap_or(0)
}

fn tints(cmds: &[DrawCmd]) -> Vec<(Rect, Option<Color>)> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Tint { rect, bg, .. } => Some((*rect, *bg)),
            _ => None,
        })
        .collect()
}

/// The labels `f`'s hint session draws: `ctx:text(nil, ...)` is what `pane: None` records, and
/// nothing else in this harness ever draws directly on the surface.
fn labels(cmds: &[DrawCmd]) -> Vec<(u16, u16, String)> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text {
                pane: None,
                x,
                y,
                text,
                ..
            } => Some((*x, *y, text.clone())),
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

/// The mode opens on the cell the pane's own cursor is on, which is where whatever you were
/// just typing is, rather than on a corner of the screen you then have to walk back from.
#[test]
fn entering_puts_the_cursor_where_the_pane_s_cursor_is() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    sub.publish_pane_tty(
        support::one(),
        std::collections::BTreeMap::from([(
            pane(),
            neotmux_proto::PaneTty {
                cursor: (5, 3),
                ..Default::default()
            },
        )]),
    );
    enter(&sub);
    assert_eq!(tints(&frame(&sub))[0].0, Rect::new(3, 5, 1, 1));
    assert_eq!(view(&sub), 0);
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
        .content(pane(), None, neotmux_proto::screen::Want::default())
        .unwrap()
        .lines;
    assert!(!screen.contains(&"four".to_string()), "{screen:?}");

    enter(&sub);
    keys(&sub, &["k"]);
    // One line back into the history is where `four` is: the host snapshots the pane there,
    // with the cells the child wrote rather than a redraw of its text.
    assert_eq!(view(&sub), 1);
}

/// `g` walks to the oldest line the pane still holds, which is the top of the history.
#[test]
fn g_lands_on_the_oldest_line_and_capital_g_on_the_newest() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["g"]);
    // The oldest line the snapshot holds, which is as far back as the view goes.
    assert_eq!(view(&sub), 4);

    keys(&sub, &["G"]);
    let cmds = frame(&sub);
    assert_eq!(
        sub.frame_views().get(&pane()).copied().unwrap_or(0),
        0,
        "the newest line is the pane's own screen"
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
    keys(&sub, &["v", "j", "l", "l"]);
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
    let down = spans(&["l", "v", "j", "j"]);
    let up = spans(&["j", "j", "l", "v", "k", "k"]);
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
    keys(&sub, &["g", "v", "j", "j", "l", "l", "l", "l", "y"]);
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

/// The yank is what the mode was entered for, so taking the text leaves it: the keyboard goes
/// back to the pane without an Esc of its own, the way tmux's own copy mode ends.
#[test]
fn yanking_leaves_the_mode() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["v", "l", "y"]);

    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough,
        "the mode still owns the keyboard after a yank"
    );
    assert!(
        tints(&frame(&sub)).is_empty(),
        "the mode is still drawing after a yank"
    );
}

/// A yank reaching over the boundary is one region, not two: `four` is the last line of the
/// history and `five` the first of the screen.
#[test]
fn a_yank_reads_across_the_boundary_between_the_history_and_the_screen() {
    let (_dir, sub) = built(REPORT);
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["g", "j", "j", "j", "v", "j", "l", "l", "l", "y"]);
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

/// What a scrolled frame costs, stated rather than guessed: nothing but the cursor.
///
/// It used to be one erase and one text command per row, because the mode redrew the view out
/// of the lines it had taken — and a line out of the ABI is text, so the view was monochrome.
/// `ctx:view` moves that job to the host, which has the cells: no row is drawn at all, and
/// the ones that appear are the pane's own with every colour on them.
#[test]
fn a_scrolled_frame_draws_nothing_but_the_cursor() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["g"]);
    let cmds = frame(&sub);
    assert!(
        texts(&cmds).is_empty(),
        "the mode redrew the rows: {cmds:?}"
    );
    assert!(
        !cmds.iter().any(|cmd| matches!(cmd, DrawCmd::Rect { .. })),
        "the mode erased the pane: {cmds:?}"
    );
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

    keys(&sub, &["G", "v", "l", "l", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "register=[ten]"),
        "the yank took what the pane says now rather than what was selected: {said:?}"
    );
}

/// A pane printing under the mode does not move what the mode is showing. The pane is frozen
/// for the life of the mode, so the cells on the screen and the lines the cursor is being
/// dragged over are the same instant — which is the whole reason a copy mode takes a copy.
#[test]
fn a_pane_that_prints_under_the_mode_does_not_move_the_view() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    keys(&sub, &["k", "k"]);
    let before = view(&sub);
    assert!(before > 0, "k did not scroll the view");

    sub.terminals().feed(pane(), b"\r\nELEVEN\r\nTWELVE");
    assert_eq!(
        view(&sub),
        before,
        "the pane printing under the mode moved what it was showing"
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
    keys(&sub, &["g", "v", "G", "l", "l", "y"]);
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
    keys(&sub, &["g", "v", "G"]);
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
    keys(&sub, &["v", "l", "y"]);
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

/// The word motions, read back through a yank: `w` steps to the next word, `$` to the last
/// one on the line, `^` back to the first. Columns, not bytes — the line is measured the way
/// the renderer measured it.
#[test]
fn the_word_motions_land_where_vim_would_put_them() {
    let (_dir, sub) = built_named(REPORT, std::rc::Rc::new(WorkingClipboard));
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), b"alpha beta gamma");
    enter(&sub);

    // From column 0: two words forward is `gamma`, and `$` is its last column.
    keys(&sub, &["w", "w", "v", "$", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    assert!(
        logged(&sub).iter().any(|line| line == "register=[gamma]"),
        "w w selected the wrong run"
    );

    // And back: `^` is the first word, `e` the end of it.
    enter(&sub);
    keys(&sub, &["$", "^", "v", "e", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    assert!(
        logged(&sub).iter().any(|line| line == "register=[alpha]"),
        "^ and e did not bracket the first word"
    );
}

/// A motion is in columns, so a line of wide characters steps by two cells a character and
/// the selection lands on the character rather than half of it.
#[test]
fn a_word_motion_counts_columns_and_not_bytes() {
    let (_dir, sub) = built_named(REPORT, std::rc::Rc::new(WorkingClipboard));
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), "日本語 ok".as_bytes());
    enter(&sub);

    keys(&sub, &["w", "v", "$", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    assert!(
        logged(&sub).iter().any(|line| line == "register=[ok]"),
        "a word motion over wide characters landed in the wrong column"
    );
}

/// `f` opens a hint session over every word the mode is showing, and typing the label under
/// one moves the cursor there — the EasyMotion half of the same module `C-b f` uses, asked a
/// different question: a position to land on rather than a token to act on.
#[test]
fn f_in_copy_mode_jumps_the_cursor_to_a_hinted_word() {
    let (_dir, sub) = built("");
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), b"alpha beta gamma");
    enter(&sub);

    keys(&sub, &["f"]);
    let marks = labels(&frame(&sub));
    assert_eq!(marks.len(), 3, "{marks:?}");
    // "alpha " is six columns, ascii, so "beta" starts at column 6.
    let beta = marks
        .iter()
        .find(|mark| mark.0 == 6)
        .unwrap_or_else(|| panic!("no label landed on beta: {marks:?}"));

    keys(&sub, &[&beta.2]);
    let cmds = frame(&sub);
    assert_eq!(
        tints(&cmds)[0].0,
        Rect::new(6, 0, 1, 1),
        "the cursor did not land on the hinted word"
    );
}

/// The line a hint answers with is a row on the screen; the cursor it moves counts against the
/// buffer. Scroll the view first, so the two disagree, and a jump that forgot `state.top` lands
/// on the row above or below the word it was asked for instead of the word itself.
#[test]
fn f_in_copy_mode_lands_on_the_buffer_line_behind_a_scrolled_view() {
    let (_dir, sub) = built_named(REPORT, std::rc::Rc::new(WorkingClipboard));
    scrolled(&sub);
    enter(&sub);
    // Scroll the view up two lines: it now shows three, four, five, six, seven, eight, with
    // "six" the fourth row — row index 3 — rather than the sixth line of the buffer.
    keys(&sub, &["k", "k"]);

    keys(&sub, &["f"]);
    let marks = labels(&frame(&sub));
    let six = marks
        .iter()
        .find(|mark| mark.1 == 3)
        .unwrap_or_else(|| panic!("no label on six's row: {marks:?}"));

    keys(&sub, &[&six.2]);
    keys(&sub, &["v", "$", "y"]);
    sub.dispatch_tick(support::one());
    keys(&sub, &["C-b", "R"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "register=[six]"),
        "the jump used the screen row as the buffer line instead of adding state.top: {said:?}"
    );
}

/// `f`'s finder answers `{ col, text }` for every run on the line, and `text` is dead on this
/// path — only `line` and `col` ever move the cursor — but a wrong one left in is worse than
/// none, so it is checked directly here rather than left for the first caller who reads it to
/// find out the hard way. Six runs, deliberately: two multi-column words, a lone `.`, a lone
/// wide character with nothing ASCII about it, a one-column word right after it with no gap,
/// and a run at the very end of the line — with a two-space gap ahead of every one of the
/// middle four, so a `finish` that crept past its own run's last column into the gap shows up
/// immediately in the text rather than only in a length nobody checked.
///
/// The spy sits in front of `hints.open` rather than reading `plugin.copy` internals directly:
/// user layers load before the shipped one, so wrapping `neotmux.hints.open` here is in effect
/// before `plugin.copy` ever requires the same, cached module table, and every `find` `f`
/// builds — production code, unmodified — passes through it on the way to the real thing.
#[test]
fn the_finder_stops_each_word_at_its_own_last_column() {
    let (_dir, sub) = built(
        "local hints = require('neotmux.hints')\n\
         local real_open = hints.open\n\
         hints.open = function(opts)\n\
           if opts.find then\n\
             for _, hit in ipairs(opts.find()) do\n\
               neotmux.log.info('hit col=' .. hit.col .. ' text=[' .. hit.text .. ']')\n\
             end\n\
           end\n\
           return real_open(opts)\n\
         end\n",
    );
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals()
        .feed(pane(), "  foo.bar  あa baz".as_bytes());
    enter(&sub);

    keys(&sub, &["f"]);
    let said = logged(&sub);
    assert_eq!(
        said,
        vec![
            "hit col=2 text=[foo]",
            "hit col=5 text=[.]",
            "hit col=6 text=[bar]",
            "hit col=11 text=[あ]",
            "hit col=13 text=[a]",
            "hit col=15 text=[baz]",
        ],
        "a run's text reached past its own last column into the gap after it"
    );
}

/// Two keys of the test's own, bound in its own layer so they are offered the key before copy
/// mode is and answer even while the mode owns the keyboard: one says whether the pane is being
/// held still, the other holds it still.
const FROZEN: &str = "neotmux.on_key(function(key)\n\
     if key == 'Q' then\n\
       neotmux.log.info('frozen=' .. tostring(neotmux.pane.frozen(1)))\n\
       return true\n\
     end\n\
     if key == 'Z' then\n\
       neotmux.pane.freeze(1)\n\
       return true\n\
     end\n\
     return false\n\
   end)\n";

/// `C-b f` while copy mode's own `f` pick is up opens a second hint session over the first.
/// The first one is gone either way; what copy mode must not do is keep yielding the keyboard
/// to a session that no longer exists — that is a mode nothing but a focus change can leave,
/// with every keystroke going into a pane the mode is drawing over.
#[test]
fn a_prefix_hint_opened_over_copy_modes_own_pick_still_leaves_copy_mode_reachable() {
    let (_dir, sub) = built(FROZEN);
    sub.terminals().open(pane(), SIZE, 100);
    // A url, so that `C-b f` — whose finders are the url/path/hash/ip set and not copy mode's
    // words — has something of its own to label.
    sub.terminals().feed(pane(), b"a http://e.com b");
    enter(&sub);

    keys(&sub, &["f"]);
    assert!(!labels(&frame(&sub)).is_empty(), "no pick to pre-empt");

    // `plugin.hints`' own session, opened over copy mode's.
    keys(&sub, &["C-b", "f"]);
    assert!(
        !labels(&frame(&sub)).is_empty(),
        "the second session drew nothing, so the keys below prove nothing"
    );
    // Its labels reach it, and not copy mode's own bindings.
    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );

    // Copy mode has its keyboard back, and leaves when told to.
    assert_eq!(sub.dispatch_key(support::one(), "j"), KeyOutcome::Consumed);
    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough,
        "copy mode never let go of the keyboard"
    );
    keys(&sub, &["Q"]);
    assert!(
        logged(&sub).iter().any(|line| line == "frozen=false"),
        "the pane was left frozen after everything closed"
    );
}

/// The mode takes the pane and the keyboard, so a hint session drawn over that pane ends as
/// the mode is entered: its labels would otherwise sit there unreadable and unreachable, and
/// the freeze it is holding is not one this mode could rely on — the host's freeze is a flag
/// and not a count, and the session would thaw the pane for everybody the moment it closed.
#[test]
fn entering_the_mode_ends_a_hint_session_drawn_over_the_pane() {
    let (_dir, sub) = built(FROZEN);
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), b"see http://e.com");

    keys(&sub, &["C-b", "f"]);
    assert!(!labels(&frame(&sub)).is_empty(), "nothing got labelled");

    enter(&sub);
    assert!(
        labels(&frame(&sub)).is_empty(),
        "the hint session is still drawing over a pane copy mode has taken"
    );
    keys(&sub, &["Q"]);
    assert!(
        logged(&sub).iter().any(|line| line == "frozen=true"),
        "copy mode did not take the pane's freeze for itself"
    );

    // One Esc, because there is only one thing left up.
    keys(&sub, &["Esc"]);
    keys(&sub, &["Q"]);
    assert!(
        logged(&sub).iter().any(|line| line == "frozen=false"),
        "copy mode left without thawing the pane"
    );
}

/// A pane held still by somebody else is not this mode's to let go of: it freezes what it
/// finds live and thaws only what it froze.
#[test]
fn leaving_does_not_thaw_a_pane_the_mode_did_not_freeze() {
    let (_dir, sub) = built(FROZEN);
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), b"anything");

    keys(&sub, &["Z"]);
    enter(&sub);
    keys(&sub, &["Esc"]);
    keys(&sub, &["Q"]);
    assert!(
        logged(&sub).iter().any(|line| line == "frozen=true"),
        "copy mode thawed a pane it never froze"
    );
}

/// A reload with copy mode open leaves the pane moving again: the new copy mode never
/// entered anything, so only the old one's `unload` can thaw what it froze.
#[test]
fn a_reload_with_copy_mode_open_leaves_no_pane_frozen() {
    let (_dir, sub) = built("");
    scrolled(&sub);
    enter(&sub);
    assert_eq!(
        sub.terminals().is_frozen(pane()),
        Some(true),
        "copy mode did not freeze the pane"
    );
    sub.reload(Some(support::one()));
    assert_eq!(
        sub.terminals().is_frozen(pane()),
        Some(false),
        "the reload left the pane frozen"
    );
}

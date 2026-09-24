//! `plugin.sidebar` and its sections, driven without a host: sessions named and framed the way
//! the server frames them, a tick for the one being looked at, and the band read back off the
//! draw commands as lines of text.

use std::collections::BTreeMap;

use neotmux_proto::{
    DrawCmd, Epoch, Geometry, PaneId, PaneInfo, Rect, SessionId, Size, Window, WindowId,
};

mod support;

use support::Built;

const SURFACE: Size = Size { rows: 12, cols: 80 };

/// One session as the host would publish it: its windows left to right, each a list of pane
/// ids, and the pane with focus. Pane ids are unique across sessions, as the server's are.
struct Sess {
    id: u32,
    name: &'static str,
    windows: Vec<Vec<u32>>,
    focus: u32,
    /// Panes that report `agent` as their command; every other pane reports `cat`.
    agents: Vec<u32>,
}

impl Sess {
    fn new(id: u32, name: &'static str, windows: Vec<Vec<u32>>) -> Self {
        let focus = windows[0][0];
        Self {
            id,
            name,
            windows,
            focus,
            agents: Vec::new(),
        }
    }
}

/// The shipped runtime with only the sidebar on, and no agent rulesets: a test about the tree
/// must not depend on whether the shipped Claude rules happen to match a blank `cat` pane. The
/// agent tests install a ruleset of their own in `extra`.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    support::built(&format!(
        "require('neotmux.defaults').sidebar = true\n\
         require('neotmux.detect').agents = {{}}\n\
         {extra}"
    ))
}

fn open(sub: &Built, sessions: &[Sess]) {
    for s in sessions {
        for pane in s.windows.iter().flatten() {
            sub.terminals().open(PaneId::from(*pane), SURFACE, 0);
        }
    }
}

fn frame_in(sub: &Built, surface: Size, sessions: &[Sess], current: u32) -> Vec<String> {
    let reserved = sub.reserved();
    let content = Size {
        rows: surface.rows - reserved.rows(),
        cols: surface.cols - reserved.cols(),
    };
    let area = Rect::new(reserved.left, reserved.top, content.cols, content.rows);
    for s in sessions {
        let id = SessionId::from(s.id);
        sub.name_session(id, s.name);
        sub.lay_out_reservations(id, surface, reserved);
        let focus = PaneId::from(s.focus);
        let at = s
            .windows
            .iter()
            .position(|w| w.contains(&s.focus))
            .expect("focus is in one of the windows");
        let shown: Vec<PaneId> = s.windows[at].iter().map(|p| PaneId::from(*p)).collect();
        let windows = s
            .windows
            .iter()
            .enumerate()
            .map(|(n, panes)| Window {
                id: WindowId::from(s.id * 100 + n as u32),
                panes: panes.iter().map(|p| PaneId::from(*p)).collect(),
                name: None,
            })
            .collect();
        sub.begin_frame(
            id,
            Epoch::new(content, shown.clone(), Some(focus)),
            Geometry {
                area,
                panes: shown.iter().map(|p| (*p, area)).collect(),
                focus: Some(focus),
                windows,
                window: Some(WindowId::from(s.id * 100 + at as u32)),
            },
        );
        let info: BTreeMap<PaneId, PaneInfo> = s
            .windows
            .iter()
            .flatten()
            .map(|p| {
                let command = if s.agents.contains(p) { "agent" } else { "cat" };
                (
                    PaneId::from(*p),
                    PaneInfo {
                        command: Some(command.to_string()),
                        cwd: None,
                    },
                )
            })
            .collect();
        sub.publish_pane_info(id, info);
    }
    sub.dispatch_tick(SessionId::from(current));
    band(sub, surface)
}

fn frame(sub: &Built, sessions: &[Sess], current: u32) -> Vec<String> {
    frame_in(sub, SURFACE, sessions, current)
}

/// Two frames: the first asks for the width the rows want, and `resize_to` lands on the next.
fn settled(sub: &Built, sessions: &[Sess], current: u32) -> Vec<String> {
    frame(sub, sessions, current);
    frame(sub, sessions, current)
}

/// Every `Text` command placed at its column, one `char` per column (true of everything the
/// sidebar draws), each line's trailing blanks trimmed.
fn band(sub: &Built, surface: Size) -> Vec<String> {
    let mut grid = vec![vec![' '; surface.cols as usize]; surface.rows as usize];
    for cmd in sub.frame_output() {
        if let DrawCmd::Text { x, y, text, .. } = cmd {
            for (i, ch) in text.chars().enumerate() {
                if let Some(cell) = grid
                    .get_mut(y as usize)
                    .and_then(|row| row.get_mut(x as usize + i))
                {
                    *cell = ch;
                }
            }
        }
    }
    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
        .collect()
}

fn line_with(lines: &[String], needle: &str) -> usize {
    lines
        .iter()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?}:\n{}", lines.join("\n")))
}

#[test]
fn the_tree_is_what_the_shipped_sidebar_draws() {
    let (_dir, sub) = built("");
    let s = [Sess::new(1, "work", vec![vec![101, 102]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "work");
    line_with(&lines, "1.1 cat");
    line_with(&lines, "1.2 cat");
}

#[test]
fn a_section_added_from_init_lua_draws_under_its_title() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.sidebar').sections, {\n\
           title = 'mine',\n\
           build = function() return { rows = { { { text = 'hello' } } } } end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let tree = line_with(&lines, "work");
    let header = line_with(&lines, "\u{2500} mine \u{2500}");
    assert!(tree < header, "{lines:#?}");
    assert_eq!(lines[header + 1], "hello", "{lines:#?}");
}

#[test]
fn a_section_with_no_rows_takes_no_room_and_draws_no_title() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.sidebar').sections, {\n\
           title = 'mine',\n\
           build = function() return { rows = {} } end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert!(!lines.iter().any(|l| l.contains("mine")), "{lines:#?}");
}

#[test]
fn a_section_that_raises_is_skipped_and_named_once() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.sidebar').sections, 1, {\n\
           title = 'broken',\n\
           build = function() error('boom') end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    settled(&sub, &s, 1);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "work");
    let said: Vec<String> = support::notified(&sub)
        .into_iter()
        .filter(|t| t.contains("section 'broken'"))
        .collect();
    assert_eq!(said.len(), 1, "{said:?}");
}

/// 12 rows, wants of 3 and 31: the capped section is held to 6, the uncapped one takes its 3,
/// and the 3 left over go back to the capped one — a header and eight rows.
#[test]
fn a_capped_section_takes_its_share_and_then_what_is_left() {
    let (_dir, sub) = built(
        "local sidebar = require('plugin.sidebar')\n\
         local function rows(prefix, n)\n\
           local out = {}\n\
           for i = 1, n do out[i] = { { text = prefix .. i } } end\n\
           return out\n\
         end\n\
         local function first(model, height)\n\
           local out = {}\n\
           for i = 1, height do out[i] = model.rows[i] end\n\
           return out\n\
         end\n\
         sidebar.sections = {\n\
           { build = function() return { rows = rows('a', 3) } end },\n\
           { title = 'b', share = 0.5, fit = first,\n\
             build = function() return { rows = rows('b', 30) } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert_eq!(&lines[0..3], &["a1", "a2", "a3"], "{lines:#?}");
    assert!(lines[3].starts_with("\u{2500} b \u{2500}"), "{lines:#?}");
    let b: Vec<String> = (1..=8).map(|i| format!("b{i}")).collect();
    assert_eq!(&lines[4..12], b.as_slice(), "{lines:#?}");
}

#[test]
fn a_share_is_not_applied_when_everything_fits() {
    let (_dir, sub) = built(
        "local sidebar = require('plugin.sidebar')\n\
         local function rows(prefix, n)\n\
           local out = {}\n\
           for i = 1, n do out[i] = { { text = prefix .. i } } end\n\
           return out\n\
         end\n\
         sidebar.sections = {\n\
           { build = function() return { rows = rows('a', 3) } end },\n\
           { title = 'b', share = 0.5, build = function() return { rows = rows('b', 8) } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let b: Vec<String> = (1..=8).map(|i| format!("b{i}")).collect();
    assert_eq!(&lines[4..12], b.as_slice(), "{lines:#?}");
}

/// Review focus 1: a row wider than the widest band is cut at the band, not drawn into the panes.
#[test]
fn a_row_wider_than_the_widest_band_is_cut_at_it() {
    let (_dir, sub) = built(
        "require('plugin.sidebar').sections = {\n\
           { build = function() return { rows = { { { text = string.rep('x', 100) } } } } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert_eq!(lines[0], "x".repeat(36), "{lines:#?}");
}

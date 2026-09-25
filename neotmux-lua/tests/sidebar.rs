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
    /// The directory every pane of the session reports, or none.
    cwd: Option<String>,
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
            cwd: None,
        }
    }

    fn focused(mut self, pane: u32) -> Self {
        self.focus = pane;
        self
    }

    fn agents(mut self, panes: &[u32]) -> Self {
        self.agents = panes.to_vec();
        self
    }
}

/// The shipped runtime with only the sidebar on, and no agent rulesets: a test about the tree
/// must not depend on whether the shipped Claude rules happen to match a blank `cat` pane. The
/// agent tests install a ruleset of their own in `extra`.
///
/// The tick budget is off, for `chime.rs`'s reason: it is eight milliseconds of wall clock, and
/// a debug build walking thirty panes across ten sessions overruns it on a loaded machine — the
/// frame is then discarded and the band comes back blank. These tests are about what the sidebar
/// draws, not about what the budget interrupts.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    built_as(TREE, extra)
}

/// The sidebar these tests were written against before herdr's became the default: the tree of
/// the session you are in, and agent rows that say where and how long. They are about how the
/// container shares the band and how those two sections cut their rows, and both sections are
/// still shipped, so they are built by name here.
const TREE: &str = "local sidebar = require('plugin.sidebar')\n\
    sidebar.sections = { require('plugin.sidebar.sessions'), require('plugin.sidebar.agents') }\n\
    sidebar.icons = { working = '\u{25cf}', blocked = '\u{25c9}', done = '\u{2713}', idle = '\u{25cb}' }\n\
    local agents = require('plugin.sidebar.agents')\n\
    agents.rows = { { 'state_icon', 'location', 'agent', 'age' } }\n\
    agents.sort = 'priority'\n";

/// The shipped sidebar as it is: herdr's spaces and agents.
fn built_herdr(extra: &str) -> (tempfile::TempDir, Built) {
    built_as("", extra)
}

fn built_herdr_running(
    extra: &str,
    runner: std::rc::Rc<support::FakeRunner>,
) -> (tempfile::TempDir, Built) {
    support::built_running(&preamble("", extra), runner)
}

fn preamble(layout: &str, extra: &str) -> String {
    format!(
        "require('neotmux.defaults').sidebar = true\n\
         require('neotmux.detect').agents = {{}}\n\
         neotmux.opt.tick_budget_ms = 0\n\
         {layout}\
         {extra}"
    )
}

fn built_as(layout: &str, extra: &str) -> (tempfile::TempDir, Built) {
    support::built(&preamble(layout, extra))
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
        // The session being looked at has the one client; every other is nobody's.
        sub.publish_attached(id, usize::from(s.id == current));
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
                        cwd: s.cwd.clone(),
                    },
                )
            })
            .collect();
        sub.publish_pane_info(id, info);
    }
    // What the server does every frame before the tick: hand each session what its jobs said.
    for s in sessions {
        sub.dispatch_jobs(SessionId::from(s.id));
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
    // Without the rule down the band's right edge, which every open row ends in and no
    // assertion here is about.
    grid.into_iter()
        .map(|row| {
            let line = row.into_iter().collect::<String>();
            let line = line.trim_end();
            line.strip_suffix('\u{2502}')
                .unwrap_or(line)
                .trim_end()
                .to_string()
        })
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
    assert_eq!(lines[0], "x".repeat(35), "{lines:#?}");
}

/// `sidebar.scroll` on a section with no fit of its own: 20 rows centred on the tenth in 12,
/// rows 4..15, with the first and last of those given over to what was cut.
#[test]
fn the_default_fit_says_how_much_it_cut_on_each_side() {
    let (_dir, sub) = built(
        "local rows = {}\n\
         for i = 1, 20 do rows[i] = { { text = 'r' .. i } } end\n\
         require('plugin.sidebar').sections = {\n\
           { build = function() return { rows = rows, anchor = 10 } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert_eq!(lines[0], "\u{2191} 4", "{lines:#?}");
    assert_eq!(lines[1], "r5", "{lines:#?}");
    assert_eq!(lines[10], "r14", "{lines:#?}");
    assert_eq!(lines[11], "\u{2193} 6", "{lines:#?}");
    line_with(&lines, "r10");
}

/// Step 2: four sessions of two rows each leave four rows for a subtree of 21, centred on the
/// focused pane (row 6 of it): rows 4..7, the first and last of those given to the cut.
#[test]
fn every_session_stays_when_the_panes_do_not_fit() {
    let (_dir, sub) = built("");
    let s = [
        Sess::new(1, "alpha", vec![vec![101]]),
        Sess::new(2, "bravo", vec![(201..=220).collect()]).focused(205),
        Sess::new(3, "charlie", vec![vec![301]]),
        Sess::new(4, "delta", vec![vec![401]]),
    ];
    open(&sub, &s);
    let lines = settled(&sub, &s, 2);
    for name in ["alpha", "bravo", "charlie", "delta"] {
        line_with(&lines, name);
    }
    let above = line_with(&lines, "\u{2026} 4 more");
    let focused = line_with(&lines, "1.5 cat");
    let below = line_with(&lines, "\u{2026} 15 more");
    assert!(line_with(&lines, "bravo") < above, "{lines:#?}");
    assert!(above < focused && focused < below, "{lines:#?}");
    assert!(below < line_with(&lines, "charlie"), "{lines:#?}");
}

/// Step 3: ten sessions of two rows are more than twelve, so each is cut to its first row and
/// the subtree goes — every name is still there, and nothing says anything was cut.
#[test]
fn ten_sessions_with_a_crowd_of_panes_keep_every_name() {
    let (_dir, sub) = built("");
    let names = [
        "s00", "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09",
    ];
    let s: Vec<Sess> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let id = n as u32 + 1;
            let panes: Vec<u32> = if *name == "s03" {
                (1..=20).map(|p| id * 100 + p).collect()
            } else {
                vec![id * 100 + 1]
            };
            Sess::new(id, name, vec![panes])
        })
        .collect();
    open(&sub, &s);
    let lines = settled(&sub, &s, 4);
    for name in names {
        line_with(&lines, name);
    }
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("more") || l.contains('\u{2191}') || l.contains('\u{2193}')),
        "{lines:#?}"
    );
}

/// Step 4: twenty one-row sessions in twelve rows, centred on the eleventh.
#[test]
fn more_sessions_than_rows_scroll_around_the_one_you_are_in() {
    let (_dir, sub) = built("");
    let names = [
        "s00", "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09", "s10", "s11", "s12",
        "s13", "s14", "s15", "s16", "s17", "s18", "s19",
    ];
    let s: Vec<Sess> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let id = n as u32 + 1;
            Sess::new(id, name, vec![vec![id * 100 + 1]])
        })
        .collect();
    open(&sub, &s);
    let lines = settled(&sub, &s, 11);
    assert_eq!(lines[0], "\u{2191} 5", "{lines:#?}");
    assert_eq!(lines[11], "\u{2193} 5", "{lines:#?}");
    line_with(&lines, "s10");
}

/// A ruleset that knows three screens by their last line, and speaks only for panes running
/// `agent`; and a clock the test moves by three minutes on `T`.
const AGENTS: &str = "local detect = require('neotmux.detect')\n\
    detect.agents = { fake = {\n\
      claims = function(info) return info ~= nil and info.command == 'agent' end,\n\
      { id = 'ask', state = 'blocked', priority = 3, visible = 'blocked',\n\
        match = function(v) return v.lines[#v.lines] == 'ASK' end },\n\
      { id = 'busy', state = 'working', priority = 2,\n\
        match = function(v) return v.lines[#v.lines] == 'BUSY' end },\n\
      { id = 'done', state = 'idle', priority = 1, visible = 'idle',\n\
        match = function(v) return v.lines[#v.lines] == 'DONE' end },\n\
    } }\n\
    T = 0\n\
    require('plugin.sidebar.agents').clock = function() return T end\n\
    neotmux.on_key(function(key)\n\
      if key == 'T' then T = T + 180000 return true end\n\
      if key == 'S' then T = T + 10000 return true end\n\
      if key == 'B' then require('plugin.sidebar').toggle() return true end\n\
      return false\n\
    end)\n";

fn show(sub: &Built, pane: u32, text: &str) {
    sub.terminals().feed(
        PaneId::from(pane),
        format!("\x1b[2J\x1b[H{text}").as_bytes(),
    );
}

#[test]
fn agents_are_listed_across_sessions_in_state_order() {
    let (_dir, sub) = built(AGENTS);
    let s = [
        Sess::new(1, "work", vec![vec![101, 102]]).agents(&[101, 102]),
        Sess::new(2, "home", vec![vec![201, 202]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 102, "ASK");
    show(&sub, 201, "DONE");
    show(&sub, 202, "DONE");
    let lines = settled(&sub, &s, 1);
    let header = line_with(&lines, "\u{2500} agents \u{2500}");
    let blocked = line_with(&lines, "\u{25c9} work:1.2 fake  0s");
    let working = line_with(&lines, "\u{25cf} work:1.1 fake  0s");
    let idle = line_with(&lines, "\u{25cb} home:1.1 fake  0s");
    assert!(
        header < blocked && blocked < working && working < idle,
        "{lines:#?}"
    );
    assert!(!lines.iter().any(|l| l.contains("home:1.2")), "{lines:#?}");
}

#[test]
fn an_age_moves_on_with_the_clock_and_starts_over_on_a_change() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    line_with(&settled(&sub, &s, 1), "\u{25cf} work:1.1 fake  0s");
    support::keys(&sub, &["T"]);
    line_with(&settled(&sub, &s, 1), "\u{25cf} work:1.1 fake  3m");
    show(&sub, 101, "DONE");
    line_with(&settled(&sub, &s, 1), "\u{25cb} work:1.1 fake  0s");
    support::keys(&sub, &["T"]);
    line_with(&settled(&sub, &s, 1), "\u{25cb} work:1.1 fake  3m");
}

/// Review focus 5: an agent that quits leaves the list, and one that comes back is new.
#[test]
fn an_agent_that_quits_leaves_and_comes_back_new() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    line_with(&settled(&sub, &s, 1), "work:1.1 fake  0s");
    support::keys(&sub, &["T"]);
    show(&sub, 101, "$ ");
    let gone = settled(&sub, &s, 1);
    assert!(!gone.iter().any(|l| l.contains("agents")), "{gone:#?}");
    support::keys(&sub, &["T"]);
    show(&sub, 101, "BUSY");
    line_with(&settled(&sub, &s, 1), "work:1.1 fake  0s");
}

#[test]
fn no_agents_no_section() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let lines = settled(&sub, &s, 1);
    assert!(!lines.iter().any(|l| l.contains("agents")), "{lines:#?}");
}

/// Review focus 3: twelve agents in twelve rows. The list is held to half — a header and five,
/// the last of them `↓ 8` — and the tree keeps its session name in the other half.
#[test]
fn more_agents_than_half_the_band_stop_at_half() {
    let (_dir, sub) = built(AGENTS);
    let panes: Vec<u32> = (101..=112).collect();
    let s = [Sess::new(1, "work", vec![panes.clone()]).agents(&panes)];
    open(&sub, &s);
    for pane in &panes {
        show(&sub, *pane, "BUSY");
    }
    let lines = settled(&sub, &s, 1);
    assert!(lines.iter().any(|l| l.ends_with(" work")), "{lines:#?}");
    let header = line_with(&lines, "\u{2500} agents \u{2500}");
    assert_eq!(header, 6, "{lines:#?}");
    assert_eq!(lines[11], "\u{2193} 8", "{lines:#?}");
    let listed = lines.iter().filter(|l| l.contains("work:1.")).count();
    assert_eq!(listed, 4, "{lines:#?}");
}

/// Review focus 4: folded, the band is one column whatever the agent list has in it.
#[test]
fn a_folded_sidebar_is_one_column_with_agents_listed() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    settled(&sub, &s, 1);
    support::keys(&sub, &["B"]);
    let lines = settled(&sub, &s, 1);
    assert!(lines.iter().all(|l| l.chars().count() <= 1), "{lines:#?}");
    assert!(lines.iter().any(|l| l == "\u{25cf}"), "{lines:#?}");
}

/// Review focus 2: two rows. The agent list gets one, which is not room for its header and a
/// row, so it draws nothing; the tree gets the other and shows its session. Nothing is raised.
#[test]
fn a_terminal_too_short_for_a_header_draws_what_fits_and_raises_nothing() {
    let (_dir, sub) = built(AGENTS);
    let tiny = Size { rows: 2, cols: 80 };
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    frame_in(&sub, tiny, &s, 1);
    let lines = frame_in(&sub, tiny, &s, 1);
    assert!(lines[0].ends_with(" work"), "{lines:#?}");
    assert_eq!(lines[1], "", "{lines:#?}");
    let said: Vec<String> = support::notified(&sub)
        .into_iter()
        .filter(|t| t.contains("plugin.sidebar"))
        .collect();
    assert!(said.is_empty(), "{said:?}");
}

/// The band's width follows its widest row, and an agent row can be it: an age that grows a
/// digit must not widen the band, or every pane is resized each time a count rolls over.
#[test]
fn an_age_rolling_over_a_digit_does_not_move_the_band() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "workspace-long", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    settled(&sub, &s, 1);
    let before = sub.reserved().left;
    support::keys(&sub, &["S"]);
    settled(&sub, &s, 1);
    assert_eq!(
        sub.reserved().left,
        before,
        "the band moved when 0s became 10s"
    );
}

/// An agent is named by the ruleset that answered, not by the process the pane reports — which
/// is whatever the agent was started through.
#[test]
fn an_agent_row_names_the_ruleset_not_the_wrapper() {
    let (_dir, sub) = built(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "work:1.1 fake");
}

/// Icons that name their state, so a line says which one it is drawn in.
const LETTERS: &str = "require('plugin.sidebar').icons = { working = 'W', blocked = 'B', done = 'D', idle = 'I' }\n\
    require('plugin.sidebar.spaces').rows = { { 'state_icon', 'name' } }\n";

/// The shipped sidebar is herdr's: a `spaces` header over every session, each drawn as the most
/// urgent of its panes, and the agents under them in herdr's rows — where on the first line,
/// what on the second — in session order, with the order named in the header.
#[test]
fn the_shipped_sidebar_is_herdrs_spaces_over_its_agents() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}"));
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201], vec![202]]).agents(&[201, 202]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 201, "ASK");
    show(&sub, 202, "BUSY");
    let lines = settled(&sub, &s, 1);
    let spaces = line_with(&lines, "\u{2500} spaces \u{2500}");
    let home = line_with(&lines, " B home");
    let work = line_with(&lines, " W work");
    let agents = line_with(&lines, "\u{2500} agents \u{2500}");
    assert!(spaces < home && home < work && work < agents, "{lines:#?}");
    assert!(lines[agents].ends_with("grouped"), "{lines:#?}");
    // Grouped: home's two before work's one, and home names its windows because it has two.
    let first = line_with(&lines, "B home 1");
    let second = line_with(&lines, "W home 2");
    let third = agents + line_with(&lines[agents..], "W work");
    assert!(
        agents < first && first < second && second < third,
        "{lines:#?}"
    );
    assert_eq!(lines[first + 1].trim(), "fake", "{lines:#?}");
}

/// herdr's `done`: an agent that finishes where nobody is looking is done until it is looked at,
/// and one that finishes on screen is only idle.
#[test]
fn an_agent_that_finishes_unwatched_is_done_until_it_is_looked_at() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}"));
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 201, "BUSY");
    settled(&sub, &s, 1);
    show(&sub, 101, "DONE");
    show(&sub, 201, "DONE");
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "I work");
    line_with(&lines, "D home");
    // Going there is looking at it.
    let lines = settled(&sub, &s, 2);
    line_with(&lines, "I home");
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "I home");
}

/// `C-b w`'s navigate: the cursor starts on the session you are in, `j` moves it, Enter goes to
/// the session under it and nothing else does.
#[test]
fn navigate_moves_a_cursor_over_the_spaces_and_enter_goes_there() {
    let (_dir, sub) = built_herdr(
        "neotmux.on_key(function(key)\n\
           if key == 'W' then require('plugin.sidebar.spaces').navigate() return true end\n\
           return false\n\
         end)\n",
    );
    let s = [
        Sess::new(1, "alpha", vec![vec![101]]),
        Sess::new(2, "bravo", vec![vec![201]]),
        Sess::new(3, "charlie", vec![vec![301]]),
    ];
    open(&sub, &s);
    settled(&sub, &s, 1);
    support::keys(&sub, &["W", "j", "j"]);
    let switches = |sub: &Built| -> Vec<String> {
        sub.drain_ops(Some(support::one()))
            .into_iter()
            .filter_map(|(_, op)| match op {
                neotmux_lua::host::Op::SessionSwitch { name } => Some(name),
                _ => None,
            })
            .collect()
    };
    assert!(switches(&sub).is_empty(), "moving the cursor switched");
    support::keys(&sub, &["Enter"]);
    assert_eq!(switches(&sub), vec!["charlie".to_string()]);
    // And the cursor is gone: a `j` now is the pane's again.
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        neotmux_lua::KeyOutcome::Fallthrough
    );
}

/// The second row of a space is its git branch and how far that is from its upstream, asked of
/// git in the directory its focused pane is in: `rev-parse` for the name, then `rev-list` for
/// behind and ahead, in that order.
#[test]
fn a_space_in_a_git_repository_shows_its_branch_and_how_far_it_is_from_upstream() {
    use neotmux_lua::host::JobChunk;
    let runner = support::FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(b"trunk\n".to_vec()),
        JobChunk::Exited(0),
    ]]);
    runner.will_say(vec![vec![
        JobChunk::Stdout(b"1\t2\n".to_vec()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built_herdr_running("", runner.clone());
    let mut work = Sess::new(1, "work", vec![vec![101]]);
    work.cwd = Some("/src/work".to_string());
    let s = [work];
    open(&sub, &s);
    let mut lines = Vec::new();
    for _ in 0..6 {
        lines = frame(&sub, &s, 1);
    }
    let started: Vec<Vec<String>> = runner
        .started
        .borrow()
        .iter()
        .map(|job| job.cmd.clone())
        .collect();
    assert_eq!(
        started[0],
        [
            "git",
            "-C",
            "/src/work",
            "rev-parse",
            "--abbrev-ref",
            "HEAD"
        ]
    );
    assert_eq!(
        started[1][3..],
        ["rev-list", "--left-right", "--count", "@{upstream}...HEAD"]
    );
    line_with(&lines, "trunk \u{2191}2 \u{2193}1");
}

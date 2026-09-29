//! `plugin.herdr.sidebar` and its sections, driven without a host: sessions named and framed the way
//! the server frames them, a tick for the one being looked at, and the band read back off the
//! draw commands as lines of text.

use std::collections::BTreeMap;

use neotmux_proto::{
    DrawCmd, Epoch, Event, Geometry, PaneId, PaneInfo, Rect, SessionId, Size, Window, WindowId,
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
    /// Panes that report `agent` as their command; every other pane reports `command`.
    agents: Vec<u32>,
    command: &'static str,
    /// The directory every pane of the session reports, or none.
    cwd: Option<String>,
    /// Each window's name, left to right; a window past the end has none.
    window_names: Vec<&'static str>,
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
            command: "cat",
            cwd: None,
            window_names: Vec::new(),
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

    fn command(mut self, command: &'static str) -> Self {
        self.command = command;
        self
    }

    fn window_names(mut self, names: &[&'static str]) -> Self {
        self.window_names = names.to_vec();
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
/// still shipped, so they are built by name here, in the band they were written against: wider,
/// shared out rather than cut at half, no fold corner, and agents only when there are some.
const TREE: &str = "local sidebar = require('plugin.herdr.sidebar')\n\
    sidebar.sections = { require('plugin.herdr.sidebar.sessions'), require('plugin.herdr.sidebar.agents') }\n\
    sidebar.widths = { min = 27, max = 36, closed = 1 }\n\
    sidebar.split, sidebar.folds = nil, nil\n\
    require('plugin.herdr.sidebar.agents').always = false\n\
    sidebar.icons = { working = '\u{25cf}', blocked = '\u{25c9}', done = '\u{2713}', idle = '\u{25cb}' }\n\
    local agents = require('plugin.herdr.sidebar.agents')\n\
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
    frame_sized(sub, &vec![surface; sessions.len()], sessions, current)
}

/// A frame in which each session has a surface of its own, `surfaces[i]` for `sessions[i]`; the
/// band read back is the current session's.
fn frame_sized(sub: &Built, surfaces: &[Size], sessions: &[Sess], current: u32) -> Vec<String> {
    frame_attached(sub, surfaces, sessions, current, &[current])
}

/// `frame_sized` with a client at every session in `attached`, and nobody at the rest.
fn frame_attached(
    sub: &Built,
    surfaces: &[Size],
    sessions: &[Sess],
    current: u32,
    attached: &[u32],
) -> Vec<String> {
    let reserved = sub.reserved();
    for (s, surface) in sessions.iter().zip(surfaces) {
        let surface = *surface;
        let content = Size {
            rows: surface.rows - reserved.rows(),
            cols: surface.cols - reserved.cols(),
        };
        let area = Rect::new(reserved.left, reserved.top, content.cols, content.rows);
        let id = SessionId::from(s.id);
        sub.name_session(id, s.name);
        sub.publish_attached(id, usize::from(attached.contains(&s.id)));
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
                name: s.window_names.get(n).map(|name| name.to_string()),
            })
            .collect();
        sub.begin_frame(
            id,
            Epoch::new(content, shown.clone(), Some(focus)),
            Geometry {
                area,
                panes: shown.iter().map(|p| (*p, area)).collect(),
                outer: Vec::new(),
                focus: Some(focus),
                windows,
                window: Some(WindowId::from(s.id * 100 + at as u32)),
                zoomed: None,
            },
        );
        let info: BTreeMap<PaneId, PaneInfo> = s
            .windows
            .iter()
            .flatten()
            .map(|p| {
                let command = if s.agents.contains(p) {
                    "agent"
                } else {
                    s.command
                };
                (
                    PaneId::from(*p),
                    PaneInfo {
                        command: Some(command.to_string()),
                        cwd: s.cwd.clone(),
                        job: None,
                    },
                )
            })
            .collect();
        sub.publish_pane_info(id, info);
    }
    // What the server does every frame: hand each session what its jobs said, and tick every
    // session whether or not anybody is attached — the one being looked at last, so the frame
    // read back is its.
    for s in sessions {
        sub.dispatch_jobs(SessionId::from(s.id));
    }
    // The bands are laid out again right before each session's tick, as the server does in
    // each session's own frame: where a band lands is the last layout's, whichever session's.
    let at = sessions
        .iter()
        .position(|s| s.id == current)
        .expect("the current session is one of them");
    let order = (0..sessions.len()).filter(|i| *i != at).chain([at]);
    for i in order {
        let id = SessionId::from(sessions[i].id);
        sub.lay_out_reservations(id, surfaces[i], reserved);
        sub.dispatch_tick(id);
    }
    LOOKED_AT.set(current);
    band(sub, surfaces[at])
}

thread_local! {
    /// The session the last frame read back was for.
    static LOOKED_AT: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
}

/// What the session the last frame was read back for is shown.
fn output(sub: &Built) -> Vec<DrawCmd> {
    output_for(sub, LOOKED_AT.get())
}

fn output_for(sub: &Built, session: u32) -> Vec<DrawCmd> {
    sub.frame_over(SessionId::from(session), &sub.frame_immediate())
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
    for cmd in output(sub) {
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

fn logged_builds(sub: &Built) -> u32 {
    support::logged(sub)
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("builds=").and_then(|n| n.parse().ok()))
        .expect("builds= logged")
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

/// A pane's command too long for the band ends in an ellipsis, in its own row and in its
/// session's, where the pane count after it stays.
#[test]
fn a_pane_command_too_long_for_the_band_ends_in_an_ellipsis() {
    let (_dir, sub) =
        built("require('plugin.herdr.sidebar').widths = { min = 20, max = 20, closed = 1 }\n");
    let s = [Sess::new(1, "work", vec![vec![101]]).command("a-command-far-too-long")];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "  a-command-far\u{2026} 1p");
    line_with(&lines, "1.1 a-comma\u{2026}");
}

#[test]
fn a_section_added_from_init_lua_draws_under_its_title() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.herdr.sidebar').sections, {\n\
           title = 'mine',\n\
           build = function() return { rows = { { { text = 'hello' } } } } end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let tree = line_with(&lines, "work");
    let header = line_with(&lines, " mine");
    assert!(tree < header, "{lines:#?}");
    assert_eq!(lines[header + 2], "hello", "{lines:#?}");
}

#[test]
fn a_section_with_no_rows_takes_no_room_and_draws_no_title() {
    let (_dir, sub) = built(
        "table.insert(require('plugin.herdr.sidebar').sections, {\n\
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
        "table.insert(require('plugin.herdr.sidebar').sections, 1, {\n\
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

/// 12 rows, wants of 3 and 33: the capped section is held to 6, the uncapped one takes its 3,
/// and the 3 left over go back to the capped one — a line, a title, a blank and six rows.
#[test]
fn a_capped_section_takes_its_share_and_then_what_is_left() {
    let (_dir, sub) = built(
        "local sidebar = require('plugin.herdr.sidebar')\n\
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
    assert_eq!(lines[3], "\u{2500}".repeat(26), "{lines:#?}");
    assert_eq!(lines[4], " b", "{lines:#?}");
    let b: Vec<String> = (1..=6).map(|i| format!("b{i}")).collect();
    assert_eq!(&lines[6..12], b.as_slice(), "{lines:#?}");
}

#[test]
fn a_share_is_not_applied_when_everything_fits() {
    let (_dir, sub) = built(
        "local sidebar = require('plugin.herdr.sidebar')\n\
         local function rows(prefix, n)\n\
           local out = {}\n\
           for i = 1, n do out[i] = { { text = prefix .. i } } end\n\
           return out\n\
         end\n\
         sidebar.sections = {\n\
           { build = function() return { rows = rows('a', 3) } end },\n\
           { title = 'b', share = 0.5, build = function() return { rows = rows('b', 6) } end },\n\
         }",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let b: Vec<String> = (1..=6).map(|i| format!("b{i}")).collect();
    assert_eq!(&lines[6..12], b.as_slice(), "{lines:#?}");
}

/// Review focus 1: a row wider than the widest band is cut at the band, not drawn into the panes.
#[test]
fn a_row_wider_than_the_widest_band_is_cut_at_it() {
    let (_dir, sub) = built(
        "require('plugin.herdr.sidebar').sections = {\n\
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
         require('plugin.herdr.sidebar').sections = {\n\
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
    require('plugin.herdr.sidebar.agents').clock = function() return T end\n\
    neotmux.on_key(function(key)\n\
      if key == 'T' then T = T + 180000 return true end\n\
      if key == 'S' then T = T + 10000 return true end\n\
      if key == 'B' then require('plugin.herdr.sidebar').toggle() return true end\n\
      return false\n\
    end)\n";

/// A pane's screen replaced, and the `pane_changed` the server raises for it in the session it is
/// in: every pane id here is its session's id times a hundred, plus its place.
fn show(sub: &Built, pane: u32, text: &str) {
    sub.terminals().feed(
        PaneId::from(pane),
        format!("\x1b[2J\x1b[H{text}").as_bytes(),
    );
    sub.dispatch_event(
        SessionId::from(pane / 100),
        &Event::PaneChanged {
            pane: PaneId::from(pane),
        },
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
    let header = line_with(&lines, " agents");
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
    let header = line_with(&lines, " agents");
    assert_eq!(header, 7, "{lines:#?}");
    assert_eq!(lines[11], "\u{2193} 10", "{lines:#?}");
    let listed = lines.iter().filter(|l| l.contains("work:1.")).count();
    assert_eq!(listed, 2, "{lines:#?}");
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
        .filter(|t| t.contains("plugin.herdr.sidebar"))
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
const LETTERS: &str = "require('plugin.herdr.sidebar').icons = { working = 'W', blocked = 'B', done = 'D', idle = 'I' }\n\
    require('plugin.herdr.sidebar.spaces').rows = { { 'state_icon', 'name' } }\n";

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
    // Tall enough that the lower half holds all three agents.
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let lines = frame_in(&sub, tall, &s, 1);
    let spaces = line_with(&lines, " spaces");
    let home = line_with(&lines, " B home");
    let work = line_with(&lines, " W work");
    let agents = line_with(&lines, " agents");
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
    assert_eq!(lines[first + 1].trim(), "blocked fake", "{lines:#?}");
}

/// A wide character is two columns, so a branch of them is cut by what it takes on screen and not
/// by how many characters it has: the counts after it stay.
#[test]
fn a_branch_of_wide_characters_is_cut_by_its_columns() {
    use neotmux_lua::host::JobChunk;
    let runner = support::FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(
            "feature/\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30d6}\u{30e9}\u{30f3}\u{30c1}\n"
                .as_bytes()
                .to_vec(),
        ),
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
    line_with(&lines, "feature/\u{65e5}\u{672c}\u{8a9e}\u{2026}");
    // `band` puts a character a cell, so where the count lands is read off the draws themselves:
    // three columns in, fifteen of branch, one of space.
    let up = output(&sub)
        .into_iter()
        .find_map(|cmd| match cmd {
            DrawCmd::Text { x, text, .. } if text == "\u{2191}2" => Some(x),
            _ => None,
        })
        .expect("the ahead count is drawn");
    assert_eq!(up, 19, "{lines:#?}");
}

/// A window name in the tree under the space you are in ends in an ellipsis when it is too long
/// for the band.
#[test]
fn a_window_name_too_long_for_the_band_ends_in_an_ellipsis() {
    let (_dir, sub) = built_herdr("");
    let s = [Sess::new(1, "work", vec![vec![101]]).window_names(&["a-window-name-far-too-long"])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    line_with(
        &lines,
        "   \u{25b8} \u{2514}\u{2500} 1 a-window-name-\u{2026}",
    );
}

/// An agent's session name too long for the band ends in an ellipsis, and its window number after
/// it stays on screen.
#[test]
fn an_agent_whose_session_name_is_too_long_ends_in_an_ellipsis() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}"));
    let s = [Sess::new(
        1,
        "a-session-name-far-too-long-for-it",
        vec![vec![101], vec![102]],
    )
    .agents(&[101, 102])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 102, "BUSY");
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let lines = frame_in(&sub, tall, &s, 1);
    let agents = line_with(&lines, " agents");
    line_with(&lines[agents..], " W a-session-name-far-\u{2026} 1");
    line_with(&lines[agents..], " W a-session-name-far-\u{2026} 2");
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
           if key == 'W' then require('plugin.herdr.sidebar.spaces').navigate() return true end\n\
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

/// A branch or a session name too long for the band ends in an ellipsis, and the counts after the
/// branch stay on screen rather than being cut off with its end.
#[test]
fn a_name_or_branch_too_long_for_the_band_ends_in_an_ellipsis() {
    use neotmux_lua::host::JobChunk;
    let runner = support::FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(b"feature/sidebar-branch-ellipsis\n".to_vec()),
        JobChunk::Exited(0),
    ]]);
    runner.will_say(vec![vec![
        JobChunk::Stdout(b"1\t2\n".to_vec()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built_herdr_running("", runner.clone());
    let mut work = Sess::new(1, "a-session-name-far-too-long-for-it", vec![vec![101]]);
    work.cwd = Some("/src/work".to_string());
    let s = [work];
    open(&sub, &s);
    let mut lines = Vec::new();
    for _ in 0..6 {
        lines = frame(&sub, &s, 1);
    }
    line_with(&lines, "   feature/sidebar\u{2026} \u{2191}2 \u{2193}1");
    line_with(&lines, " a-session-name-far-to\u{2026}");
}

/// With the palette on and the sidebar off (the defaults), `C-b g` still knows an agent finished
/// unwatched: `done` is recorded by `neotmux.seen`'s own tick, not by whoever draws it.
const PALETTE_ONLY: &str = "local defaults = require('neotmux.defaults')\n\
    defaults.sidebar = false\n\
    defaults.palette = true\n\
    neotmux.on_key(function(key)\n\
      if key == 'G' then require('plugin.palette').open('#') return true end\n\
      return false\n\
    end)\n";

#[test]
fn an_agent_that_finished_unwatched_is_done_in_the_palette_with_no_sidebar() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{PALETTE_ONLY}"));
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 201, "BUSY");
    frame(&sub, &s, 1);
    show(&sub, 201, "DONE");
    frame(&sub, &s, 1);
    support::keys(&sub, &["G"]);
    let lines = frame(&sub, &s, 1);
    line_with(&lines, "home fake done");
}

/// And one you watched finish is not done, whatever the palette last saw when it was open.
#[test]
fn an_agent_watched_as_it_finished_is_not_done_in_the_palette() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{PALETTE_ONLY}"));
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    frame(&sub, &s, 1);
    support::keys(&sub, &["G"]);
    frame(&sub, &s, 1);
    support::keys(&sub, &["Esc"]);
    show(&sub, 101, "DONE");
    frame(&sub, &s, 1);
    // Over to home, and the palette opened from there: work is nobody's screen any more.
    frame(&sub, &s, 2);
    assert_eq!(
        sub.dispatch_key(SessionId::from(2), "G"),
        neotmux_lua::KeyOutcome::Consumed
    );
    let lines = frame(&sub, &s, 2);
    line_with(&lines, "work fake idle");
}

/// A session is the most urgent of all its panes, not only the window it is showing.
#[test]
fn a_space_counts_the_agents_in_its_hidden_windows() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}"));
    let s = [Sess::new(1, "work", vec![vec![101], vec![102]]).agents(&[102])];
    open(&sub, &s);
    show(&sub, 102, "ASK");
    let lines = settled(&sub, &s, 1);
    line_with(&lines, " B work");
}

/// Nothing the sidebar shows changed, so a tick does not rewrite its buffer: the host keeps
/// drawing the lines it has.
#[test]
fn a_tick_with_nothing_new_does_not_rebuild_the_sidebar() {
    let (_dir, sub) = built_herdr(
        "local sidebar = require('plugin.herdr.sidebar')\n\
         local builds = 0\n\
         local real = sidebar.render\n\
         sidebar.render = function(...) builds = builds + 1 return real(...) end\n\
         neotmux.on_key(function(key) if key == 'B' then neotmux.log.info('builds=' .. builds) return true end return false end)\n",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    settled(&sub, &s, 1);
    let lines = frame(&sub, &s, 1);
    support::keys(&sub, &["B"]);
    let first: u32 = logged_builds(&sub);
    assert!(first > 0, "the sidebar was never rendered: {lines:#?}");
    for _ in 0..5 {
        frame(&sub, &s, 1);
    }
    support::keys(&sub, &["B"]);
    assert_eq!(logged_builds(&sub), first, "{lines:#?}");
    line_with(&lines, "work");
}

/// Every session ticks, but the one buffer is the sidebar of the session a client is at: a tick in
/// a session nobody is at neither rewrites it with its own "you are here" nor rewrites it at all.
#[test]
fn a_session_nobody_is_at_does_not_rebuild_the_sidebar() {
    let (_dir, sub) = built_herdr(
        "local sidebar = require('plugin.herdr.sidebar')\n\
         local builds = 0\n\
         local real = sidebar.render\n\
         sidebar.render = function(...) builds = builds + 1 return real(...) end\n\
         neotmux.on_key(function(key) if key == 'B' then neotmux.log.info('builds=' .. builds) return true end return false end)\n",
    );
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    open(&sub, &s);
    settled(&sub, &s, 1);
    frame(&sub, &s, 1);
    support::keys(&sub, &["B"]);
    let first = logged_builds(&sub);
    let mut lines = Vec::new();
    for _ in 0..5 {
        lines = frame(&sub, &s, 1);
    }
    support::keys(&sub, &["B"]);
    assert_eq!(logged_builds(&sub), first, "{lines:#?}");
    assert_eq!(style_of(&sub, "work"), (TEXT, true), "{lines:#?}");
    assert_eq!(style_of(&sub, "home"), (SUBTEXT, false), "{lines:#?}");
}

/// A client at each of two sessions: each session's band is its own sidebar, with its own "you
/// are here", whichever session ticked last, and with nothing new neither tick rewrites it.
#[test]
fn two_sessions_with_a_client_each_show_their_own_sidebar() {
    let (_dir, sub) = built_herdr(
        "local sidebar = require('plugin.herdr.sidebar')\n\
         local builds = 0\n\
         local real = sidebar.render\n\
         sidebar.render = function(...) builds = builds + 1 return real(...) end\n\
         neotmux.on_key(function(key) if key == 'B' then neotmux.log.info('builds=' .. builds) return true end return false end)\n",
    );
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    let sizes = [SURFACE, SURFACE];
    open(&sub, &s);
    for _ in 0..2 {
        frame_attached(&sub, &sizes, &s, 1, &[1, 2]);
        frame_attached(&sub, &sizes, &s, 2, &[1, 2]);
    }
    support::keys(&sub, &["B"]);
    let first = logged_builds(&sub);
    for _ in 0..5 {
        frame_attached(&sub, &sizes, &s, 1, &[1, 2]);
        frame_attached(&sub, &sizes, &s, 2, &[1, 2]);
    }
    support::keys(&sub, &["B"]);
    assert_eq!(
        logged_builds(&sub),
        first,
        "idle alternating ticks rendered"
    );
    // Session 1 ticked last; what session 2 is shown is still its own.
    frame_attached(&sub, &sizes, &s, 1, &[1, 2]);
    assert_eq!(style_in(&sub, 2, "home"), (TEXT, true));
    assert_eq!(style_in(&sub, 2, "work"), (SUBTEXT, false));
    assert_eq!(style_in(&sub, 1, "work"), (TEXT, true));
    assert_eq!(style_in(&sub, 1, "home"), (SUBTEXT, false));
}

/// Sessions of different sizes: the band is laid out for the one a client is at, and a tick in
/// the other, whose band is another height, neither rewrites it nor makes the next one rewrite it.
#[test]
fn a_session_of_another_size_does_not_rebuild_the_sidebar() {
    let (_dir, sub) = built_herdr(
        "local sidebar = require('plugin.herdr.sidebar')\n\
         local builds = 0\n\
         local real = sidebar.render\n\
         sidebar.render = function(...) builds = builds + 1 return real(...) end\n\
         neotmux.on_key(function(key) if key == 'B' then neotmux.log.info('builds=' .. builds) return true end return false end)\n",
    );
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    let sizes = [Size { rows: 20, cols: 80 }, Size { rows: 12, cols: 80 }];
    open(&sub, &s);
    frame_sized(&sub, &sizes, &s, 1);
    frame_sized(&sub, &sizes, &s, 1);
    support::keys(&sub, &["B"]);
    let first = logged_builds(&sub);
    let mut lines = Vec::new();
    for _ in 0..5 {
        lines = frame_sized(&sub, &sizes, &s, 1);
    }
    support::keys(&sub, &["B"]);
    assert_eq!(logged_builds(&sub), first, "{lines:#?}");
    assert_eq!(lines[10], "\u{2500}".repeat(25), "{lines:#?}");
    assert_eq!(lines[19].trim(), "\u{ab}", "{lines:#?}");
}

/// The host samples what a pane runs apart from its output, so a command that changed after the
/// last output is in the tree the frame `pane_info_changed` says so, with no timer.
#[test]
fn a_command_that_changes_with_no_output_is_shown_the_next_frame() {
    let (_dir, sub) = built("");
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    line_with(&settled(&sub, &s, 1), "1.1 cat");
    let s = [Sess::new(1, "work", vec![vec![101]]).command("vim")];
    line_with(&frame(&sub, &s, 1), "1.1 vim");
}

/// A ruleset whose `QUIET` is a fall to idle the screen does not say in so many words, so the
/// detector holds it `working` for a settle short enough to wait out.
const SETTLING: &str = "local detect = require('neotmux.detect')\n\
    detect.settle = 200\n\
    detect.agents = { fake = {\n\
      claims = function(info) return info ~= nil and info.command == 'agent' end,\n\
      { id = 'busy', state = 'working', priority = 2,\n\
        match = function(v) return v.lines[#v.lines] == 'BUSY' end },\n\
      { id = 'quiet', state = 'idle', priority = 1,\n\
        match = function(v) return v.lines[#v.lines] == 'QUIET' end },\n\
    } }\n";

/// A `working` pane that settles with no output after its last is shown idle the first frame
/// after the settle: `neotmux.seen` asks a working pane again every frame.
#[test]
fn a_working_agent_that_settles_with_no_output_is_shown_idle_the_frame_after_the_settle() {
    let (_dir, sub) = built(SETTLING);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    line_with(&settled(&sub, &s, 1), "\u{25cf} work:1.1 fake");
    show(&sub, 101, "QUIET");
    line_with(&frame(&sub, &s, 1), "\u{25cf} work:1.1 fake");
    std::thread::sleep(std::time::Duration::from_millis(500));
    sub.dispatch_timers(
        SessionId::from(1),
        std::time::Instant::now() + std::time::Duration::from_secs(2),
    );
    line_with(&frame(&sub, &s, 1), "\u{25cb} work:1.1 fake");
}

/// The session that claimed the sidebar's timer ending before it ever fired does not take the
/// clock with it: the next session to frame claims a new one, and it still asks git again.
#[test]
fn the_timer_outlives_the_session_that_claimed_it() {
    use neotmux_lua::host::JobChunk;
    let runner = support::FakeRunner::new();
    for _ in 0..16 {
        runner.will_say(vec![vec![JobChunk::Exited(1)]]);
    }
    let (_dir, sub) = built_herdr_running(
        "require('plugin.herdr.sidebar.spaces').refresh = 0\n",
        runner.clone(),
    );
    let mut work = Sess::new(1, "work", vec![vec![101]]);
    work.cwd = Some("/src/work".to_string());
    let mut home = Sess::new(2, "home", vec![vec![201]]);
    home.cwd = Some("/src/home".to_string());
    let both = [work, home];
    open(&sub, &both);
    settled(&sub, &both, 1);
    let now = std::time::Instant::now();
    sub.dispatch_timers(SessionId::from(1), now);
    sub.dispatch_event_unbound(&Event::SessionClosed {
        session: "work".to_string(),
    });
    sub.forget_session(SessionId::from(1));
    let mut home = Sess::new(2, "home", vec![vec![201]]);
    home.cwd = Some("/src/home".to_string());
    let home = [home];
    for _ in 0..3 {
        frame(&sub, &home, 2);
    }
    let asked = runner.started.borrow().len();
    let now = std::time::Instant::now();
    sub.dispatch_timers(SessionId::from(2), now);
    sub.dispatch_timers(SessionId::from(2), now + std::time::Duration::from_secs(2));
    let started = runner.started.borrow();
    assert!(started.len() > asked, "git was not asked again");
    assert_eq!(started[asked].cmd[..3], ["git", "-C", "/src/home"]);
}

/// A window named since the last render is in the tree the frame after `window_renamed` says so.
#[test]
fn a_window_renamed_is_in_the_sidebar_the_next_frame() {
    let (_dir, sub) = built_herdr("");
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    settled(&sub, &s, 1);
    let s = [Sess::new(1, "work", vec![vec![101]]).window_names(&["editor"])];
    sub.dispatch_event(
        SessionId::from(1),
        &Event::WindowRenamed {
            window: WindowId::from(100),
        },
    );
    let lines = frame(&sub, &s, 1);
    line_with(&lines, "1 editor");
}

/// A pane whose output changes its state rewrites that session's row.
#[test]
fn output_that_changes_a_panes_state_rewrites_the_sidebar() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}"));
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let lines = settled(&sub, &s, 1);
    line_with(&lines, " W work");
    show(&sub, 101, "ASK");
    sub.dispatch_event(
        SessionId::from(1),
        &Event::PaneChanged {
            pane: PaneId::from(101),
        },
    );
    let lines = settled(&sub, &s, 1);
    line_with(&lines, " B work");
}

/// `C-b` while the cursor is up leaves it, so the prefix key that follows is the prefix's and
/// the one after that is the pane's again.
#[test]
fn the_prefix_key_leaves_navigate() {
    let (_dir, sub) = built_herdr("require('neotmux.defaults').prefix = true\n");
    let s = [
        Sess::new(1, "alpha", vec![vec![101]]),
        Sess::new(2, "bravo", vec![vec![201]]),
    ];
    open(&sub, &s);
    settled(&sub, &s, 1);
    support::keys(&sub, &["C-b", "w"]);
    frame(&sub, &s, 1);
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        neotmux_lua::KeyOutcome::Consumed,
        "navigate did not take j"
    );
    support::keys(&sub, &["C-b"]);
    frame(&sub, &s, 1);
    support::keys(&sub, &["c"]);
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        neotmux_lua::KeyOutcome::Fallthrough,
        "the cursor was still up after the prefix"
    );
}

/// Ten directories are more than the job limit lets be asked at once; every one of them still
/// gets its branch, a few at a time, rather than the same ones failing every round.
#[test]
fn every_space_gets_its_branch_however_many_there_are() {
    use neotmux_lua::host::JobChunk;
    let runner = support::FakeRunner::new();
    for _ in 0..40 {
        runner.will_say(vec![vec![
            JobChunk::Stdout(b"trunk\n".to_vec()),
            JobChunk::Exited(0),
        ]]);
    }
    let (_dir, sub) =
        built_herdr_running("require('plugin.herdr.sidebar').widths.max = 40\n", runner);
    let names = ["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9"];
    let s: Vec<Sess> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let id = n as u32 + 1;
            let mut it = Sess::new(id, name, vec![vec![id * 100 + 1]]);
            it.cwd = Some(format!("/src/{name}"));
            it
        })
        .collect();
    open(&sub, &s);
    // Tall enough that half of it holds ten spaces of two rows each.
    let big = Size { rows: 60, cols: 80 };
    let mut lines = Vec::new();
    for _ in 0..30 {
        lines = frame_in(&sub, big, &s, 1);
    }
    let branches = lines.iter().filter(|l| l.trim() == "trunk").count();
    assert_eq!(branches, 10, "{lines:#?}");
}

/// The state is part of what `#` matches, so typing one narrows to the agents in it.
#[test]
fn typing_a_state_in_the_agents_palette_narrows_to_it() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{PALETTE_ONLY}"));
    let s = [Sess::new(1, "work", vec![vec![101, 102]]).agents(&[101, 102])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 102, "ASK");
    frame(&sub, &s, 1);
    support::keys(&sub, &["G", "b", "l", "o", "c", "k"]);
    let lines = frame(&sub, &s, 1);
    line_with(&lines, "blocked");
    assert!(!lines.iter().any(|l| l.contains("working")), "{lines:#?}");
}

// ---- herdr's look, as read off a real herdr 0.9.1 with catppuccin (2026-09-25) ----

const TEXT: neotmux_proto::Color = neotmux_proto::Color::Rgb(0xcd, 0xd6, 0xf4);
const RED: neotmux_proto::Color = neotmux_proto::Color::Rgb(0xf3, 0x8b, 0xa8);
const SUBTEXT: neotmux_proto::Color = neotmux_proto::Color::Rgb(0xa6, 0xad, 0xc8);
const OVERLAY: neotmux_proto::Color = neotmux_proto::Color::Rgb(0x6c, 0x70, 0x86);
const HERE: neotmux_proto::Color = neotmux_proto::Color::Rgb(0x1e, 0x1e, 0x2e);
const CURSOR: neotmux_proto::Color = neotmux_proto::Color::Rgb(0x31, 0x32, 0x44);

/// The first text drawn that holds `needle`: its colour and whether it is bold.
fn style_of(sub: &Built, needle: &str) -> (neotmux_proto::Color, bool) {
    style_in(sub, LOOKED_AT.get(), needle)
}

/// `style_of` in what `session` is shown.
fn style_in(sub: &Built, session: u32, needle: &str) -> (neotmux_proto::Color, bool) {
    output_for(sub, session)
        .into_iter()
        .find_map(|cmd| match cmd {
            DrawCmd::Text { text, fg, bold, .. } if text.contains(needle) => Some((fg, bold)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("nothing drawn holds {needle:?}"))
}

/// What row `y` of the band is painted with at its first column, if anything paints it.
fn fill_on(sub: &Built, y: u16) -> Option<neotmux_proto::Color> {
    paint_at(sub, 0, y)
}

/// What the cell at `x`, `y` is painted with, if anything paints it: the background of the last
/// command drawn over it, one `char` per column as everything the sidebar draws is.
fn paint_at(sub: &Built, x: u16, y: u16) -> Option<neotmux_proto::Color> {
    let mut paint = None;
    for cmd in output(sub) {
        match cmd {
            DrawCmd::Rect { rect, bg, .. }
                if rect.y <= y && y < rect.y + rect.h && rect.x <= x && x < rect.x + rect.w =>
            {
                paint = Some(bg);
            }
            DrawCmd::Text {
                x: at,
                y: row,
                text,
                bg,
                ..
            } if row == y && at <= x && usize::from(x - at) < text.chars().count() => {
                paint = (bg != neotmux_proto::Color::Default).then_some(bg);
            }
            _ => {}
        }
    }
    paint
}

/// Row for row what herdr draws: ` spaces` and a blank, the sessions two rows each — the second
/// empty where there is no branch — with the tree of the one you are in under it, then at half
/// the band a rule across it, ` agents` with the
/// order on its right and a blank, the agents one column in with their state and what they are
/// three columns in, and the fold's `«` in the corner.
#[test]
fn the_sidebar_is_laid_out_row_for_row_as_herdrs() {
    let (_dir, sub) = built_herdr(AGENTS);
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let lines = frame_in(&sub, tall, &s, 1);
    assert_eq!(lines[0], " spaces", "{lines:#?}");
    assert_eq!(lines[1], "", "{lines:#?}");
    assert_eq!(lines[2], " \u{b7} home", "{lines:#?}");
    assert_eq!(lines[3], "", "{lines:#?}");
    assert_eq!(lines[4], " \u{25cf} work", "{lines:#?}");
    assert_eq!(lines[5], "", "{lines:#?}");
    assert_eq!(lines[6], "   \u{25b8} \u{2514}\u{2500} 1", "{lines:#?}");
    assert_eq!(lines[10], "\u{2500}".repeat(25), "{lines:#?}");
    assert!(
        lines[11].starts_with(" agents") && lines[11].ends_with("grouped"),
        "{lines:#?}"
    );
    assert_eq!(lines[12], "", "{lines:#?}");
    assert_eq!(lines[13], " \u{25cf} work", "{lines:#?}");
    assert_eq!(lines[14], "   working fake", "{lines:#?}");
    assert_eq!(lines[19].trim(), "\u{ab}", "{lines:#?}");
}

/// Nothing paints the band but the rows you are in: herdr leaves the terminal's own background
/// and marks the session and the agent you are at. Names are bold where herdr's are.
#[test]
fn only_the_rows_you_are_in_are_painted_and_names_are_herdrs_colours() {
    let (_dir, sub) = built_herdr(AGENTS);
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    frame_in(&sub, tall, &s, 1);
    assert_eq!(fill_on(&sub, 2), None, "home is not where you are");
    assert_eq!(fill_on(&sub, 3), None, "nor its second row");
    assert_eq!(fill_on(&sub, 4), Some(HERE), "work is");
    assert_eq!(
        fill_on(&sub, 5),
        Some(HERE),
        "both of its rows, the empty one too"
    );
    assert_eq!(fill_on(&sub, 13), Some(HERE), "the agent you are at");
    assert_eq!(fill_on(&sub, 14), Some(HERE), "both of its rows");
    assert_eq!(style_of(&sub, "spaces"), (OVERLAY, true));
    assert_eq!(style_of(&sub, "home"), (SUBTEXT, false));
    assert_eq!(style_of(&sub, "work"), (TEXT, true));
    assert_eq!(style_of(&sub, "fake"), (OVERLAY, false));
}

/// The column the rule is drawn in on row `y`.
fn rule_at(sub: &Built, y: u16) -> u16 {
    output(sub)
        .into_iter()
        .find_map(|cmd| match cmd {
            DrawCmd::Text {
                x, y: row, text, ..
            } if row == y => text
                .chars()
                .position(|c| c == '\u{2502}')
                .map(|at| x + at as u16),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no rule on row {y}"))
}

/// The rows you are at are painted up to the rule and not under it: the rule stays on the band's
/// own colour, as it was before the sidebar was a buffer.
#[test]
fn the_rows_you_are_at_are_painted_up_to_the_rule_and_not_under_it() {
    let (_dir, sub) = built_herdr(AGENTS);
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let lines = settled(&sub, &s, 1);
    let work = line_with(&lines, " \u{25cf} work") as u16;
    let rule = rule_at(&sub, work);
    assert_eq!(paint_at(&sub, 0, work), Some(HERE), "{lines:#?}");
    assert_eq!(paint_at(&sub, rule - 1, work), Some(HERE), "{lines:#?}");
    assert_eq!(paint_at(&sub, rule, work), None, "{lines:#?}");
}

/// A section's fill naming a group of your own, one with no `:bg` twin, paints its row's whole
/// width, the rule too, as every fill did before the twins.
#[test]
fn a_fill_of_your_own_group_paints_the_whole_row() {
    let (_dir, sub) = built_herdr(
        "neotmux.hl.set('mine:row', { bg = '#0a0b0c' })\n\
         table.insert(require('plugin.herdr.sidebar').sections, {\n\
           build = function() return { rows = { { { text = 'hello' }, fill = 'mine:row' } } } end,\n\
         })",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let hello = line_with(&lines, "hello") as u16;
    let rule = rule_at(&sub, hello);
    let mine = Some(neotmux_proto::Color::Rgb(0x0a, 0x0b, 0x0c));
    assert_eq!(paint_at(&sub, 0, hello), mine, "{lines:#?}");
    assert_eq!(paint_at(&sub, rule, hello), mine, "{lines:#?}");
}

/// With a band colour of your own, the rule of a row you are at is on the band's colour.
#[test]
fn the_rule_of_a_row_you_are_at_is_on_the_band_colour() {
    let (_dir, sub) = built_herdr("require('neotmux.style').set('sidebar', { bar = '#123456' })\n");
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    let work = line_with(&lines, " work") as u16;
    let rule = rule_at(&sub, work);
    assert_eq!(paint_at(&sub, rule - 1, work), Some(HERE), "{lines:#?}");
    assert_eq!(
        paint_at(&sub, rule, work),
        Some(neotmux_proto::Color::Rgb(0x12, 0x34, 0x56)),
        "{lines:#?}"
    );
}

/// The agents are headed whether or not there are any, as herdr's are.
#[test]
fn the_agents_are_headed_with_no_agents() {
    let (_dir, sub) = built_herdr("");
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, " agents");
}

/// Folded, it is herdr's rail: every session numbered with its state, a rule, every agent
/// numbered with its state, and `»` to open it again.
#[test]
fn folded_it_is_herdrs_rail() {
    let (_dir, sub) = built_herdr(AGENTS);
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    settled(&sub, &s, 1);
    support::keys(&sub, &["B"]);
    let lines = settled(&sub, &s, 1);
    assert_eq!(lines[0], "1 \u{b7}", "{lines:#?}");
    assert_eq!(lines[1], "2 \u{25cf}", "{lines:#?}");
    assert_eq!(lines[6], "\u{2500}".repeat(3), "{lines:#?}");
    assert_eq!(lines[7], "1 \u{25cf}", "{lines:#?}");
    assert_eq!(lines[11].trim(), "\u{bb}", "{lines:#?}");
}

/// While the cursor is up the bottom row says so, as herdr's does, and the cursor's rows are
/// painted with herdr's selection.
#[test]
fn navigate_says_so_along_the_bottom_and_paints_the_cursor() {
    let (_dir, sub) = built_herdr(
        "neotmux.on_key(function(key)\n\
           if key == 'W' then require('plugin.herdr.sidebar.spaces').navigate() return true end\n\
           return false\n\
         end)\n",
    );
    let s = [
        Sess::new(1, "alpha", vec![vec![101]]),
        Sess::new(2, "bravo", vec![vec![201]]),
    ];
    open(&sub, &s);
    settled(&sub, &s, 1);
    support::keys(&sub, &["W", "j"]);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "NAVIGATE");
    let bravo = line_with(&lines, " bravo") as u16;
    assert_eq!(fill_on(&sub, bravo), Some(CURSOR), "{lines:#?}");
    support::keys(&sub, &["Esc"]);
    let lines = settled(&sub, &s, 1);
    assert!(!lines.iter().any(|l| l.contains("NAVIGATE")), "{lines:#?}");
}

/// A band colour of your own does not hide the rows you are at: the fill is what they are drawn
/// on, whatever the band is.
#[test]
fn a_band_colour_of_your_own_still_shows_the_rows_you_are_at() {
    let (_dir, sub) = built_herdr("require('neotmux.style').set('sidebar', { bar = '#181825' })\n");
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    settled(&sub, &s, 1);
    let bg = output(&sub)
        .into_iter()
        .find_map(|cmd| match cmd {
            DrawCmd::Text { text, bg, .. } if text == "work" => Some(bg),
            _ => None,
        })
        .expect("work is drawn");
    assert_eq!(bg, HERE);
}

/// `style.set('sidebar', ...)` is not only read by a row's own fill: the band itself, every row
/// a fill of its own does not paint, is the user's colour too.
#[test]
fn a_style_set_recolours_the_band_itself() {
    let (_dir, sub) = built_herdr("require('neotmux.style').set('sidebar', { bar = '#123456' })\n");
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    settled(&sub, &s, 1);
    assert_eq!(
        fill_on(&sub, 0),
        Some(neotmux_proto::Color::Rgb(0x12, 0x34, 0x56)),
        "the band itself is painted"
    );
}

/// A `bar` set after the sidebar has rendered paints the band at once, with nothing rewritten.
#[test]
fn a_bar_set_at_runtime_paints_the_band_with_no_render() {
    let (_dir, sub) = built_herdr(
        "neotmux.on_key(function(key)\n\
           if key == 'P' then require('neotmux.style').set('sidebar', { bar = '#123456' }) return true end\n\
           return false\n\
         end)\n",
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    settled(&sub, &s, 1);
    assert_eq!(fill_on(&sub, 0), None);
    support::keys(&sub, &["P"]);
    assert_eq!(
        fill_on(&sub, 0),
        Some(neotmux_proto::Color::Rgb(0x12, 0x34, 0x56))
    );
}

/// The theme, not only a plugin's own `style.set`, reaches the sidebar: `header` is `overlay0`
/// by default, and renaming what `overlay0` means recolours it with no `sidebar` selector touched
/// at all.
#[test]
fn a_theme_change_reaches_the_sidebar_too() {
    let (_dir, sub) = built_herdr("require('neotmux.theme').set({ overlay0 = '#010203' })\n");
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    frame_in(&sub, tall, &s, 1);
    assert_eq!(
        style_of(&sub, "spaces"),
        (neotmux_proto::Color::Rgb(0x01, 0x02, 0x03), true)
    );
}

/// The session you are in is opened out under its space: its windows and panes, as the tree
/// draws them, so where you are inside it is on screen too. The others stay one entry each.
#[test]
fn the_space_you_are_in_shows_its_windows_and_panes() {
    let (_dir, sub) = built_herdr("");
    let s = [
        Sess::new(1, "work", vec![vec![101, 102], vec![103]]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    open(&sub, &s);
    let tall = Size { rows: 30, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let lines = frame_in(&sub, tall, &s, 1);
    let work = line_with(&lines, " work");
    let rule = line_with(&lines, "\u{2500}\u{2500}\u{2500}");
    for pane in ["1.1", "1.2", "2.1"] {
        let at = line_with(&lines, pane);
        assert!(work < at && at < rule, "{pane}: {lines:#?}");
        assert!(lines[at].ends_with(pane), "no command after it: {lines:#?}");
    }
    assert!(
        !lines[rule..]
            .iter()
            .chain(&lines[..work])
            .any(|l| l.contains("1.1")),
        "only the session you are in is opened out: {lines:#?}"
    );
}

#[test]
fn the_tree_under_the_space_you_are_in_can_be_turned_off() {
    let (_dir, sub) = built_herdr("require('plugin.herdr.sidebar.spaces').tree = false\n");
    let s = [Sess::new(1, "work", vec![vec![101, 102]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    assert!(!lines.iter().any(|l| l.contains("1.1")), "{lines:#?}");
}

/// A session with more windows than the top half holds keeps every space's name before any of
/// its panes, and the pane you are in among the panes it keeps.
#[test]
fn a_tree_too_tall_for_the_half_keeps_the_names_and_the_pane_you_are_in() {
    let (_dir, sub) = built_herdr("");
    let windows: Vec<Vec<u32>> = (0..8).map(|w| vec![101 + w]).collect();
    let mut work = Sess::new(1, "work", windows);
    work.focus = 108;
    let s = [work, Sess::new(2, "home", vec![vec![201]])];
    open(&sub, &s);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, " home");
    line_with(&lines, " work");
    line_with(&lines, "8.1");
}

/// `C-b a`: the spaces' navigate, over the agents. The cursor starts on the agent you are at,
/// `j` moves it, and Enter goes to the one under it — its window and pane in its own session,
/// then that session.
#[test]
fn navigate_moves_a_cursor_over_the_agents_and_enter_goes_to_that_pane() {
    let (_dir, sub) = built_herdr(&format!(
        "{AGENTS}neotmux.on_key(function(key)\n\
           if key == 'A' then require('plugin.herdr.sidebar.agents').navigate() return true end\n\
           return false\n\
         end)\n"
    ));
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201], vec![202]]).agents(&[201, 202]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 201, "BUSY");
    show(&sub, 202, "ASK");
    settled(&sub, &s, 1);
    support::keys(&sub, &["A", "j", "j"]);
    let lines = settled(&sub, &s, 1);
    line_with(&lines, "NAVIGATE");
    assert!(lines.iter().any(|l| l.ends_with(" agent")), "{lines:#?}");
    sub.drain_ops(Some(support::one()));
    sub.drain_ops(Some(SessionId::from(2)));

    support::keys(&sub, &["Enter"]);
    let here = sub.drain_ops(Some(support::one()));
    let there = sub.drain_ops(Some(SessionId::from(2)));
    use neotmux_lua::host::Op;
    assert!(
        there.iter().any(|(_, op)| *op
            == Op::PaneFocus {
                pane: PaneId::from(202)
            }),
        "{there:?}"
    );
    assert!(
        here.iter().chain(there.iter()).any(|(_, op)| *op
            == Op::SessionSwitch {
                name: "home".to_string()
            }),
        "{here:?} {there:?}"
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        neotmux_lua::KeyOutcome::Fallthrough,
        "the cursor stayed up after Enter"
    );
}

/// One cursor at a time: opening the agents' puts the spaces' away, and the other way round.
#[test]
fn one_navigate_cursor_puts_the_other_away() {
    let (_dir, sub) = built_herdr(&format!(
        "{AGENTS}neotmux.on_key(function(key)\n\
           if key ~= 'X' then return false end\n\
           local spaces = require('plugin.herdr.sidebar.spaces')\n\
           local agents = require('plugin.herdr.sidebar.agents')\n\
           local function say() neotmux.log.info(tostring(spaces.cursor) .. ' ' .. tostring(agents.cursor)) end\n\
           spaces.navigate() say()\n\
           agents.navigate() say()\n\
           spaces.navigate() say()\n\
           return true\n\
         end)\n"
    ));
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    settled(&sub, &s, 1);
    sub.drain_ops(Some(support::one()));
    support::keys(&sub, &["X"]);
    let said: Vec<String> = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::host::Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(said, vec!["1 nil", "nil 1", "1 nil"]);
}

/// Counts `sidebar.render`, the whole band laid out again, and logs the count on `N`.
const RENDERS: &str = "local sidebar = require('plugin.herdr.sidebar')\n\
    local renders = 0\n\
    local real = sidebar.render\n\
    sidebar.render = function(...) renders = renders + 1 return real(...) end\n\
    neotmux.on_key(function(key) if key == 'N' then neotmux.log.info('renders=' .. renders) return true end return false end)\n";

fn renders(sub: &Built) -> u32 {
    support::keys(sub, &["N"]);
    support::logged(sub)
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("renders=").and_then(|n| n.parse().ok()))
        .expect("renders= logged")
}

/// The lines that differ between two reads of the band, as (row, before, after).
fn changed(before: &[String], after: &[String]) -> Vec<(usize, String, String)> {
    before
        .iter()
        .zip(after)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(y, (a, b))| (y, a.clone(), b.clone()))
        .collect()
}

/// One pane's state moving rewrites the rows that show it, its space's and its agent's, and
/// nothing else: the band is not laid out again, and every other row is as it was.
#[test]
fn a_panes_state_changing_rewrites_only_the_rows_that_show_it() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}{RENDERS}"));
    let s = [
        Sess::new(1, "work", vec![vec![101, 102]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 201, "BUSY");
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let before = frame_in(&sub, tall, &s, 1);
    let first = renders(&sub);
    show(&sub, 101, "ASK");
    frame_in(&sub, tall, &s, 1);
    let after = frame_in(&sub, tall, &s, 1);
    assert_eq!(renders(&sub), first, "{after:#?}");
    let spaces = line_with(&before, " W work");
    let agents = line_with(&before[spaces + 1..], " W work") + spaces + 1;
    assert_eq!(
        changed(&before, &after),
        vec![
            (spaces, " W work".to_string(), " B work".to_string()),
            (agents, " W work".to_string(), " B work".to_string()),
            (
                agents + 1,
                "   working fake".to_string(),
                "   blocked fake".to_string()
            ),
        ],
        "{after:#?}"
    );
    // The rows rewritten keep their colours and their fill.
    assert_eq!(fill_on(&sub, spaces as u16), Some(HERE), "{after:#?}");
    assert_eq!(fill_on(&sub, agents as u16 + 1), Some(HERE), "{after:#?}");
    assert_eq!(style_of(&sub, "blocked"), (RED, false), "{after:#?}");
    assert_eq!(style_of(&sub, "work"), (TEXT, true), "{after:#?}");
}

/// A row rewritten in place is painted as a laid out one is: up to the rule, not under it.
#[test]
fn a_row_rewritten_in_place_is_painted_up_to_the_rule() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}{RENDERS}"));
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let before = frame_in(&sub, tall, &s, 1);
    let first = renders(&sub);
    show(&sub, 101, "ASK");
    frame_in(&sub, tall, &s, 1);
    let after = frame_in(&sub, tall, &s, 1);
    assert_eq!(renders(&sub), first, "{after:#?}");
    let spaces = line_with(&before, " W work");
    assert_eq!(after[spaces], " B work", "{after:#?}");
    let y = spaces as u16;
    let rule = rule_at(&sub, y);
    assert_eq!(paint_at(&sub, 0, y), Some(HERE), "{after:#?}");
    assert_eq!(paint_at(&sub, rule - 1, y), Some(HERE), "{after:#?}");
    assert_eq!(paint_at(&sub, rule, y), None, "{after:#?}");
}

/// A client at each of two sessions: a state moving in one is rewritten into both bands, each
/// still its own, and neither is laid out again.
#[test]
fn a_state_change_reaches_every_attached_sessions_band_without_a_render() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}{RENDERS}"));
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    let tall = Size { rows: 20, cols: 80 };
    let sizes = [tall, tall];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 201, "BUSY");
    for _ in 0..2 {
        frame_attached(&sub, &sizes, &s, 1, &[1, 2]);
        frame_attached(&sub, &sizes, &s, 2, &[1, 2]);
    }
    let first = renders(&sub);
    show(&sub, 201, "ASK");
    frame_attached(&sub, &sizes, &s, 2, &[1, 2]);
    let one = frame_attached(&sub, &sizes, &s, 1, &[1, 2]);
    assert_eq!(renders(&sub), first, "{one:#?}");
    line_with(&one, " B home");
    assert_eq!(style_in(&sub, 1, "work"), (TEXT, true));
    assert_eq!(style_in(&sub, 1, "home"), (SUBTEXT, false));
    let two = frame_attached(&sub, &sizes, &s, 2, &[1, 2]);
    line_with(&two, " B home");
    assert_eq!(style_in(&sub, 2, "home"), (TEXT, true));
    assert_eq!(style_in(&sub, 2, "work"), (SUBTEXT, false));
}

/// A session coming is more rows, so the band is laid out again, once.
#[test]
fn a_session_added_lays_the_band_out_again() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}{RENDERS}"));
    let one = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &one);
    show(&sub, 101, "BUSY");
    settled(&sub, &one, 1);
    let first = renders(&sub);
    let two = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]),
    ];
    open(&sub, &two[1..]);
    sub.dispatch_event_unbound(&Event::SessionCreated {
        session: "home".to_string(),
    });
    let lines = settled(&sub, &two, 1);
    assert_eq!(renders(&sub), first + 1, "{lines:#?}");
    line_with(&lines, " home");
}

/// Under `priority` a state moving can move an agent past another, which is the band laid out
/// again, in the new order.
#[test]
fn a_state_change_that_reorders_the_agents_lays_the_band_out_again() {
    let (_dir, sub) = built_herdr(&format!(
        "{AGENTS}{LETTERS}{RENDERS}require('plugin.herdr.sidebar.agents').sort = 'priority'\n"
    ));
    let s = [
        Sess::new(1, "work", vec![vec![101]]).agents(&[101]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    show(&sub, 201, "BUSY");
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let lines = frame_in(&sub, tall, &s, 1);
    let header = line_with(&lines, " agents");
    assert!(
        line_with(&lines[header..], " W home") < line_with(&lines[header..], " W work"),
        "{lines:#?}"
    );
    let first = renders(&sub);
    show(&sub, 101, "ASK");
    frame_in(&sub, tall, &s, 1);
    let lines = frame_in(&sub, tall, &s, 1);
    assert_eq!(renders(&sub), first + 1, "{lines:#?}");
    assert!(
        line_with(&lines[header..], " B work") < line_with(&lines[header..], " W home"),
        "{lines:#?}"
    );
}

/// An age moving on is its rows rewritten, with the band not laid out again.
#[test]
fn an_age_moving_on_rewrites_its_rows_without_a_render() {
    let (_dir, sub) = built_herdr(&format!(
        "{AGENTS}{LETTERS}{RENDERS}require('plugin.herdr.sidebar.agents').rows = {{ {{ 'state_icon', 'session', 'age' }} }}\n"
    ));
    let s = [Sess::new(1, "work", vec![vec![101]]).agents(&[101])];
    open(&sub, &s);
    show(&sub, 101, "BUSY");
    let before = settled(&sub, &s, 1);
    let row = line_with(&before, " W work  0s");
    let first = renders(&sub);
    support::keys(&sub, &["T"]);
    let after = settled(&sub, &s, 1);
    assert_eq!(renders(&sub), first, "{after:#?}");
    assert_eq!(
        changed(&before, &after),
        vec![(row, " W work  0s".to_string(), " W work  3m".to_string())],
        "{after:#?}"
    );
}

/// In a band whose width follows its rows, a row that narrows but was not the widest leaves the
/// width where it was, so it is rewritten with the band not laid out again.
#[test]
fn a_row_narrowing_that_was_not_the_widest_is_rewritten_without_a_render() {
    let (_dir, sub) = built_herdr(&format!(
        "{AGENTS}{LETTERS}{RENDERS}require('plugin.herdr.sidebar').widths = {{ min = 20, max = 40, closed = 4 }}\n"
    ));
    let s = [
        Sess::new(1, "a-rather-long-session-name", vec![vec![101]]),
        Sess::new(2, "work", vec![vec![201]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 201, "BUSY");
    let tall = Size { rows: 20, cols: 80 };
    frame_in(&sub, tall, &s, 1);
    let before = frame_in(&sub, tall, &s, 1);
    let first = renders(&sub);
    show(&sub, 201, "DONE");
    frame_in(&sub, tall, &s, 1);
    let after = frame_in(&sub, tall, &s, 1);
    assert_eq!(renders(&sub), first, "{after:#?}");
    let row = line_with(&before, "   working fake");
    assert_eq!(after[row], "   done fake", "{after:#?}");
    line_with(&after, " a-rather-long-session-name");
}

/// Only `neotmux.seen` of everything that asks a pane its state, and a count of how often it does
/// on `C`. A shell is idle the moment it is first looked at, so the panes here have no clock.
const COUNTED: &str = "local defaults = require('neotmux.defaults')\n\
    defaults.sidebar, defaults.palette, defaults.chime = false, false, false\n\
    require('neotmux.detect').agents = {}\n\
    neotmux.opt.tick_budget_ms = 0\n\
    local state = require('neotmux.state')\n\
    state.working_for = 0\n\
    require('neotmux.seen')\n\
    local of, asked = state.of, 0\n\
    state.of = function(pane) asked = asked + 1 return of(pane) end\n\
    neotmux.on_key(function(key) if key == 'C' then neotmux.log.info('asked=' .. asked) return true end return false end)\n";

fn logged_asked(sub: &Built) -> u32 {
    support::logged(sub)
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("asked=").and_then(|n| n.parse().ok()))
        .expect("asked= logged")
}

/// `neotmux.seen` asks a pane again when something that can move it happened, not every frame:
/// idle panes nothing happens to cost it nothing, however many sessions they are in.
#[test]
fn seen_asks_nothing_of_idle_panes_nothing_happens_to() {
    let (_dir, sub) = support::built(COUNTED);
    let s = [
        Sess::new(1, "work", vec![vec![101, 102], vec![103]]),
        Sess::new(2, "home", vec![vec![201, 202]]),
    ];
    open(&sub, &s);
    for _ in 0..3 {
        frame(&sub, &s, 1);
    }
    support::keys(&sub, &["C"]);
    let first = logged_asked(&sub);
    assert!(first > 0, "seen never asked the panes");
    for _ in 0..5 {
        frame(&sub, &s, 1);
    }
    support::keys(&sub, &["C"]);
    assert_eq!(logged_asked(&sub), first);
}

/// A client attaching to a session is somebody looking at its window: a done agent there is only
/// idle from then on, with no window or focus changing.
#[test]
fn a_client_attaching_clears_done_in_the_window_it_is_shown() {
    let (_dir, sub) = built_herdr(&format!("{AGENTS}{LETTERS}"));
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 201, "BUSY");
    settled(&sub, &s, 1);
    show(&sub, 201, "DONE");
    line_with(&settled(&sub, &s, 1), "D home");
    let both = [SURFACE, SURFACE];
    frame_attached(&sub, &both, &s, 1, &[1, 2]);
    line_with(&frame_attached(&sub, &both, &s, 1, &[1, 2]), "I home");
}

/// A `working` agent that settles with no output and nobody watching is done a frame after the
/// settle, with no event and no timer: time alone moves a working pane.
#[test]
fn a_working_agent_that_settles_unwatched_is_done_with_no_event_and_no_timer() {
    let (_dir, sub) = built_herdr(&format!("{SETTLING}{PALETTE_ONLY}"));
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    open(&sub, &s);
    show(&sub, 201, "BUSY");
    frame(&sub, &s, 1);
    show(&sub, 201, "QUIET");
    frame(&sub, &s, 1);
    std::thread::sleep(std::time::Duration::from_millis(500));
    frame(&sub, &s, 1);
    support::keys(&sub, &["G"]);
    line_with(&frame(&sub, &s, 1), "home fake done");
}

/// The model's decision about a pane lands between frames with no output and no event, and the
/// sidebar shows it: `neotmux.seen` notices a decision landed.
#[test]
fn a_decision_that_lands_with_no_output_is_in_the_sidebar() {
    use neotmux_lua::host::JobChunk;
    let runner = support::FakeRunner::new();
    let answer = r#"{"model":"jev-1.13.0","answers":{"p101":{"type":"choice","choice":"blocked","probabilities":{"blocked":0.9,"working":0.05,"idle":0.05},"confidence":0.9}},"usage":{"input_tokens":300,"output_tokens":20}}
200"#;
    runner.will_say(vec![vec![
        JobChunk::Stdout(answer.as_bytes().to_vec()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = support::built_running(
        &preamble(
            TREE,
            "local jev = require('neotmux.jev')\n\
             jev.enabled, jev.settle_ms = true, 1\n\
             require('neotmux.state').working_for = 0\n",
        ),
        runner.clone(),
    );
    let s = [Sess::new(1, "work", vec![vec![101]])];
    open(&sub, &s);
    show(&sub, 101, "Select an option (1-3):");
    line_with(&settled(&sub, &s, 1), "\u{25cb} work");
    let mut lines = Vec::new();
    for _ in 0..4 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        lines = frame(&sub, &s, 1);
    }
    assert_eq!(runner.started.borrow().len(), 1, "{lines:#?}");
    line_with(&lines, "\u{25c9} work");
}

/// A pane that closes is forgotten: nothing of what it went through is kept.
#[test]
fn a_pane_that_closes_is_forgotten_by_seen() {
    let (_dir, sub) = built_herdr(&format!(
        "{AGENTS}{LETTERS}neotmux.on_key(function(key)\n\
           if key == 'M' then\n\
             local seen = require('neotmux.seen')\n\
             neotmux.log.info('moved=' .. tostring(seen.moved[202]) .. ' of=' .. seen.of(202, 'idle'))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    ));
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201, 202]]).agents(&[201, 202]),
    ];
    open(&sub, &s);
    show(&sub, 202, "BUSY");
    settled(&sub, &s, 1);
    show(&sub, 202, "DONE");
    line_with(&settled(&sub, &s, 1), "D home");
    support::keys(&sub, &["M"]);
    assert!(
        support::logged(&sub)
            .iter()
            .any(|l| l.ends_with(" of=done")),
        "202 was not done"
    );
    sub.terminals().close(PaneId::from(202));
    sub.dispatch_event(
        SessionId::from(2),
        &Event::PaneClosed {
            pane: PaneId::from(202),
        },
    );
    let s = [
        Sess::new(1, "work", vec![vec![101]]),
        Sess::new(2, "home", vec![vec![201]]).agents(&[201]),
    ];
    settled(&sub, &s, 1);
    support::keys(&sub, &["M"]);
    assert!(
        support::logged(&sub)
            .iter()
            .any(|l| l == "moved=nil of=idle"),
        "202 was not forgotten"
    );
}

//! `neotmux.detect`: what a pane looks like it is doing, as a function of what is on it.
//! The rules under test here are the tests' own — the shipped Claude ruleset has its own file.
//!
//! The harness is `hints.rs`'s: a real pane fed real bytes through `support::panes()`, with the
//! shipped plugins off so that every log line in a dispatch belongs to the test's own `on_key`.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 16, cols: 40 };

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

fn pane() -> PaneId {
    PaneId::from(1)
}

/// The shipped runtime with every plugin off. `neotmux.detect` is a convention module — nothing
/// registers it — so the only thing running here is what the test's `init.lua` asks for.
fn built(extra: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'guard', 'sync', 'watch', 'sidebar', 'footer',\n\
               'hints' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
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

fn feed(sub: &Built, text: &[u8]) {
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), text);
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        assert_eq!(
            sub.dispatch_key(support::one(), spec),
            KeyOutcome::Consumed,
            "the test's own on_key did not take '{spec}'"
        );
    }
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

/// Priority decides, not order in the table: a lower-priority rule that matches first in the
/// table must not win.
#[test]
fn the_highest_priority_matching_rule_answers() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         local rules = {\n\
           { id = 'low', state = 'idle', priority = 1, match = function() return true end },\n\
           { id = 'high', state = 'blocked', priority = 9, match = function() return true end },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local state, id = detect.judge(rules, detect.view(1))\n\
             neotmux.log.info('state=' .. tostring(state) .. ' id=' .. tostring(id))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    keys(&sub, &["J"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|l| l == "state=blocked id=high"),
        "{said:?}"
    );
}

/// A rule with no state says "this screen is not a state" — a transcript viewer, a menu — and
/// stops the search without answering, so the caller keeps whatever it had.
#[test]
fn a_rule_with_no_state_stops_the_search_without_answering() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         local rules = {\n\
           { id = 'viewer', priority = 9, match = function() return true end },\n\
           { id = 'idle', state = 'idle', priority = 1, match = function() return true end },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local state, id = detect.judge(rules, detect.view(1))\n\
             neotmux.log.info('state=' .. tostring(state) .. ' id=' .. tostring(id))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    keys(&sub, &["J"]);
    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "state=nil id=viewer"), "{said:?}");
}

/// The view's regions, against one screen that has all three: a rule above, a box below, and
/// blank lines that no region counts.
#[test]
fn the_view_names_the_regions_a_rule_reads() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local view = detect.view(1)\n\
             neotmux.log.info('tail=' .. table.concat(view.tail(2), '|'))\n\
             neotmux.log.info('after=' .. table.concat(view.after_rule(), '|'))\n\
             neotmux.log.info('box=' .. table.concat(view.prompt_box(), '|'))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(
        &sub,
        "above\r\n\
         \u{2500}\u{2500}\u{2500}\u{2500}\r\n\
         between\r\n\
         \r\n\
         \u{256d}\u{2500}\u{2500}\u{256e}\r\n\
         \u{2502} \u{276f} \u{2502}\r\n\
         \u{2570}\u{2500}\u{2500}\u{256f}"
            .as_bytes(),
    );
    keys(&sub, &["J"]);
    let said = logged(&sub);
    // The blank line is in none of them: every region is built from the non-blank lines.
    assert!(
        said.iter()
            .any(|l| l == "tail=\u{2502} \u{276f} \u{2502}|\u{2570}\u{2500}\u{2500}\u{256f}"),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.starts_with("after=between|")),
        "the rule line itself or the blank leaked into after_rule: {said:?}"
    );
    assert!(said.iter().any(|l| l == "box=\u{276f}"), "{said:?}");
}

/// A two-rule ruleset flipped by a value the test controls, so that the only thing under test
/// is the hold: a spinner that says `working`, and a quiet screen whose `visible` the caller
/// picks. Nothing calls `detect.forget` on the way to a look — forgetting a pane drops its hold
/// too, which is the very thing these are watching — so the revision is moved from the Rust
/// side instead, the way being fed moves it in the real thing.
fn holding(settle: u32, visible: Option<&str>) -> String {
    let visible = match visible {
        Some(state) => format!("visible = '{state}', "),
        None => String::new(),
    };
    format!(
        "local detect = require('neotmux.detect')\n\
         detect.settle = {settle}\n\
         local says = 'working'\n\
         detect.agents = {{\n\
           test = {{\n\
             {{ id = 'spinner', state = 'working', priority = 9, visible = 'working',\n\
               match = function() return says == 'working' end }},\n\
             {{ id = 'quiet', state = 'idle', priority = 1, {visible}\n\
               match = function() return says == 'idle' end }},\n\
           }},\n\
         }}\n\
         neotmux.on_key(function(key)\n\
           if key == 'W' then says = 'working' return true end\n\
           if key == 'I' then says = 'idle' return true end\n\
           if key == 'F' then detect.forget(1) return true end\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(detect.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    )
}

/// One look: the pane is fed, which is what moves the revision the look is cached against, and
/// then asked.
fn look(sub: &Built) -> Vec<String> {
    sub.terminals().feed(pane(), b".");
    keys(sub, &["J"]);
    logged(sub)
}

/// Held: a fall from working to plain idle is not believed at once. The rules are the test's,
/// flipped by a value it controls, so the only thing under test is the hold.
#[test]
fn a_working_pane_that_looks_idle_is_still_working_until_it_settles() {
    let (_dir, sub) = built(&holding(50, None));
    feed(&sub, b"anything");
    keys(&sub, &["W"]);
    assert!(look(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["I"]);
    assert!(
        look(&sub).iter().any(|l| l == "of=working"),
        "a working pane fell to idle the moment its spinner blinked"
    );

    // Past `settle`, with nothing new to say for itself, it is idle.
    std::thread::sleep(std::time::Duration::from_millis(60));
    assert!(look(&sub).iter().any(|l| l == "of=idle"));
}

/// Unless the screen is *visibly* idle, which is evidence rather than an absence of it.
#[test]
fn a_visible_idle_prompt_is_believed_at_once() {
    let (_dir, sub) = built(&holding(5000, Some("idle")));
    feed(&sub, b"anything");
    keys(&sub, &["W"]);
    assert!(look(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["I"]);
    assert!(
        look(&sub).iter().any(|l| l == "of=idle"),
        "a prompt on the screen is evidence, and was held anyway"
    );
}

/// `visible` is a state name and is read as one. A rule that answers `idle` while claiming to
/// show `working` — a copy-paste from the spinner rule above it — is a rule with a typo in it,
/// and must not buy its way out of the hold with it. Held is the safe half of that mistake:
/// the alternative is the spinner-blink storm the hold exists to stop, with nothing saying so.
#[test]
fn an_idle_rule_that_claims_to_show_working_is_held_like_any_other() {
    let (_dir, sub) = built(&holding(5000, Some("working")));
    feed(&sub, b"anything");
    keys(&sub, &["W"]);
    assert!(look(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["I"]);
    assert!(
        look(&sub).iter().any(|l| l == "of=working"),
        "a rule answering idle released the hold by claiming to show working"
    );
}

/// `forget` is the call for "start over on this pane", and a hold it left behind would answer
/// the next question out of a screen nobody has read.
#[test]
fn forgetting_a_pane_drops_the_hold_and_not_only_the_look() {
    let (_dir, sub) = built(&holding(5000, None));
    feed(&sub, b"anything");
    keys(&sub, &["W"]);
    assert!(look(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["I"]);
    assert!(look(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["F"]);
    assert!(
        look(&sub).iter().any(|l| l == "of=idle"),
        "the pane was still being held at working after it was forgotten"
    );
}

/// A rule that raises is reported and skipped — once. `judge` runs for every pane on every
/// frame, and a rule that raises on one screen raises on the next thousand: a notification per
/// frame is not a report, it is the pane nobody can read any more.
#[test]
fn a_raising_rule_is_reported_once_and_not_once_a_look() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         local rules = {\n\
           { id = 'bad', state = 'idle', priority = 9, match = function() error('boom') end },\n\
           { id = 'good', state = 'working', priority = 1, match = function() return true end },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local state = detect.judge(rules, detect.view(1))\n\
             neotmux.log.info('state=' .. tostring(state))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    keys(&sub, &["J", "J", "J"]);
    let said = logged(&sub);
    let complaints = said
        .iter()
        .filter(|l| l.starts_with("neotmux.detect: rule 'bad'"))
        .count();
    assert_eq!(complaints, 1, "{said:?}");
    assert_eq!(
        said.iter().filter(|l| *l == "state=working").count(),
        3,
        "the raising rule stopped the search instead of being skipped: {said:?}"
    );
}

/// One screen, one title, one answer. The title is set the way a program sets it — OSC 2 in the
/// same bytes as the screen, because opening a pane starts it over — and the answer is `judge`'s
/// own rather than `of`'s, so that no hold stands between a fixture and the rule it is about.
fn assert_detects(screen: &str, title: &str, state: &str, id: &str) {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local state, id = detect.judge(detect.agents.claude, detect.view(1))\n\
             neotmux.log.info('state=' .. tostring(state) .. ' id=' .. tostring(id))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    let mut bytes = Vec::new();
    if !title.is_empty() {
        bytes.extend_from_slice(format!("\x1b]2;{title}\x07").as_bytes());
    }
    bytes.extend_from_slice(screen.replace('\n', "\r\n").as_bytes());
    feed(&sub, &bytes);
    keys(&sub, &["J"]);
    let want = format!("state={state} id={id}");
    let said = logged(&sub);
    assert!(said.contains(&want), "want {want}: {said:?}");
}

/// The title outranks the screen: a screen that reads idle under a working title is working.
#[test]
fn the_title_outranks_the_screen() {
    let screen = "╭────────╮\n│ ❯      │\n╰────────╯\n";
    assert_detects(screen, "⠋ Claude", "working", "osc_title_working");
}

/// Scrolled back into the transcript: whatever the pane is doing, this screen is not it.
#[test]
fn a_transcript_scrolled_back_is_not_a_state() {
    let screen = "> what did you change\nShowing detailed transcript (ctrl+o)\n";
    assert_detects(screen, "", "nil", "transcript_viewer");
}

/// A form with something to confirm is blocked; "esc to cancel" on its own would not be.
#[test]
fn a_form_waiting_to_be_confirmed_is_blocked() {
    let screen = "Which branch?\nEnter to confirm · Esc to cancel\n";
    assert_detects(screen, "", "blocked", "live_blocked_form");
}

/// The `/btw` overlay is typed at a turn that is still running.
#[test]
fn a_btw_overlay_is_working() {
    let screen = "/btw the port is 8080\n";
    assert_detects(screen, "", "working", "btw_overlay_working");
}

/// A working turn: the spinner line Claude draws while it is going, with its interrupt hint.
#[test]
fn a_turn_in_progress_is_working() {
    let screen = "✻ Herding… (12s · esc to interrupt)\n";
    assert_detects(screen, "", "working", "live_turn_working");
}

/// An input box with nothing running behind it, and none of the hints a dialog comes with.
/// Both prompt characters: which one Claude Code draws has changed between versions.
#[test]
fn an_empty_prompt_box_is_idle() {
    for prompt in ["\u{276f}", ">"] {
        let screen = format!("╭────────╮\n│ {prompt}      │\n╰────────╯\n");
        assert_detects(&screen, "", "idle", "live_prompt_box");
    }
}

/// Picking a model is a menu over the top, not a state.
#[test]
fn the_model_picker_is_not_a_state() {
    let screen = "Select Model\n1. Default\nEnter to set as default, Esc to cancel\n";
    assert_detects(screen, "", "nil", "model_picker_menu");
}

/// A Bash approval names the command and offers numbered options.
#[test]
fn a_bash_permission_prompt_is_blocked() {
    let screen = "Bash command\n\
                  ls -la\n\
                  Do you want to proceed?\n\
                  ❯ 1. Yes\n\
                    2. Yes, and don't ask again for: ls\n\
                    3. No\n";
    assert_detects(screen, "", "blocked", "bash_permission_prompt");
}

/// Any other approval: the question and a numbered answer, with nothing naming a command.
#[test]
fn a_generic_permission_prompt_is_blocked() {
    let screen = "Edit src/main.rs\n\
                  Do you want to proceed?\n\
                  1. Yes\n\
                  2. No\n\
                  Esc to cancel\n";
    assert_detects(screen, "", "blocked", "generic_permission_prompt");
}

/// The backstop: words that read as a question left standing, and no prompt to point at.
#[test]
fn a_screen_waiting_for_permission_is_blocked() {
    let screen = "Waiting for permission to continue\n";
    assert_detects(screen, "", "blocked", "legacy_no_prompt_blocker");
}

/// What Claude Code leaves in the title when a turn is over.
#[test]
fn an_idle_title_is_idle() {
    assert_detects("all done\n", "✳ Claude", "idle", "osc_title_idle");
}

/// The shape a permission prompt is really drawn in: a rounded box, with the hint line below it
/// rather than inside it. Our `prompt_box()` is the last rounded box — herdr's was narrower —
/// so without an exclusion of its own `live_prompt_box` outranks this at 950 and calls a dialog
/// waiting on you idle. Lines are cut to the test pane's forty columns.
#[test]
fn a_permission_prompt_inside_a_box_is_blocked_and_not_an_idle_prompt() {
    let screen = "╭────────────────────────────────────╮\n\
                  │ Bash command                       │\n\
                  │ ls -la                             │\n\
                  │                                    │\n\
                  │ Do you want to proceed?            │\n\
                  │ ❯ 1. Yes                           │\n\
                  │   2. Yes, don't ask again          │\n\
                  │   3. No, tell Claude what to do    │\n\
                  ╰────────────────────────────────────╯\n\
                  \x20 Esc to cancel\n";
    assert_detects(screen, "", "blocked", "bash_permission_prompt");
}

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

/// The view's regions, against one screen that has all of them: a rule with something under it,
/// a prompt between the last two rules, and blank lines that no region counts. The shape is the
/// one a real Claude Code draws — `╭…╰` is a box and not a rule, so nothing here is one.
#[test]
fn the_view_names_the_regions_a_rule_reads() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local view = detect.view(1)\n\
             neotmux.log.info('tail=' .. table.concat(view.tail(5), '|'))\n\
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
         \u{2500}\u{2500}\u{2500}\u{2500}\r\n\
         \u{276f}\r\n\
         \u{2500}\u{2500}\u{2500}\u{2500}\r\n\
         status"
            .as_bytes(),
    );
    keys(&sub, &["J"]);
    let said = logged(&sub);
    // The blank line is in none of them: every region is built from the non-blank lines, so the
    // last five are the five a program wrote and not four and an empty row.
    assert!(
        said.iter().any(|l| l
            == "tail=between|\u{2500}\u{2500}\u{2500}\u{2500}|\u{276f}|\u{2500}\u{2500}\u{2500}\u{2500}|status"),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l == "after=status"),
        "the rule line itself leaked into after_rule: {said:?}"
    );
    assert!(said.iter().any(|l| l == "box=\u{276f}"), "{said:?}");
}

/// A screen with one rule on it has no prompt box: that is what a dialog looks like, and the
/// region coming back empty rather than guessing is what keeps a rule from reading one as a
/// prompt nobody has typed into.
#[test]
fn a_screen_with_a_single_rule_has_no_prompt_box() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('box=' .. #detect.view(1).prompt_box())\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(
        &sub,
        "\u{2500}\u{2500}\u{2500}\u{2500}\r\n\
         \u{276f} 1. Yes\r\n\
         \x20 2. No"
            .as_bytes(),
    );
    keys(&sub, &["J"]);
    assert!(logged(&sub).iter().any(|l| l == "box=0"));
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
///
/// The pane is wider and taller than the rest of this file's. Wider because wrapping one of
/// Claude Code's rules onto a second line would hand the view two rules where the screen has
/// one; taller because the captured permission screen is seventeen lines and the default pane
/// is sixteen, and a fixture that scrolled off its own top would be testing the scrollback.
fn assert_detects(screen: &str, title: &str, state: &str, id: &str) {
    const CAPTURED: Size = Size {
        rows: 40,
        cols: 100,
    };
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
    sub.terminals().open(pane(), CAPTURED, 100);
    if !title.is_empty() {
        sub.terminals()
            .feed(pane(), format!("\x1b]2;{title}\x07").as_bytes());
    }
    sub.terminals()
        .feed(pane(), screen.replace('\n', "\r\n").as_bytes());
    keys(&sub, &["J"]);
    let want = format!("state={state} id={id}");
    let said = logged(&sub);
    assert!(said.contains(&want), "want {want}: {said:?}");
}

/// The rule Claude Code draws around its prompt, at the width the captures were taken at.
const BAR: &str = "────────────────────────────────────────────────────────────────────────────";

/// The lighter separators Claude Code draws around a diff. Not rules: a different character.
const DASHED: &str = "╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌";

/// Captured: idle, nothing running. The prompt is a `❯` between two rules, with the status
/// lines under the lower one.
fn idle_screen() -> String {
    format!(
        "✻ Cogitated for 6s · done 9:01 PM\n\
         \n\
         {BAR}\n\
         ❯\n\
         {BAR}\n\
         \x20 ⚠ Transcript saving is off …\n\
         \x20 [Opus 5.5 (1M context)] cmp ●─────────── 7% | ctx …\n\
         \x20 -- INSERT -- ⏵⏵ auto mode on (shift+tab to cycle)\n"
    )
}

/// Captured: a turn running. The same prompt is still on the screen under the spinner line.
fn working_screen() -> String {
    format!(
        "\x20 Listing 1 directory… (ctrl+o to expand)\n\
         \x20 ⎿  $ ls -la /etc/hostname\n\
         \n\
         * Actioning… (3s · ↓ 16 tokens)\n\
         \n\
         {BAR}\n\
         ❯\n\
         {BAR}\n"
    )
}

/// Captured: a Bash permission prompt. One rule, and the dialog runs to the bottom of the
/// screen — no second rule, so no prompt box, so nothing reads this as an idle prompt.
fn permission_screen() -> String {
    format!(
        "\x20 Reading 1 file… (ctrl+o to expand)\n\
         \x20 ⎿  $ cat /etc/hostname\n\
         \n\
         {BAR}\n\
         \x20Bash command\n\
         \x20Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below\n\
         \n\
         \x20  cat /etc/hostname\n\
         \x20  Print hostname\n\
         \n\
         \x20Do you want to proceed?\n\
         \x20❯ 1. Yes\n\
         \x20  2. Yes, allow reading from /etc from this project\n\
         \x20  3. Yes, and switch to auto mode · auto mode handles these prompts for you\n\
         \x20  4. No\n\
         \n\
         \x20Esc to cancel · Tab to amend\n"
    )
}

/// Captured: the trust dialog Claude Code opens on its first run in a directory.
fn trust_screen() -> &'static str {
    "  Security guide\n\
     \n\
     ❯ No, exit\n\
     \x20 Yes, I trust this folder\n\
     \n\
     \x20 Enter to confirm · Esc to cancel\n"
}

/// Captured: the title spins a half-circle while a turn runs, and outranks the screen — which
/// on this very screen still shows the prompt the pane is not waiting at.
#[test]
fn the_title_outranks_the_screen() {
    assert_detects(
        &working_screen(),
        "◐ hostname ファイルの確認",
        "working",
        "osc_title_working",
    );
}

/// Captured: the transcript after ctrl+o. Whatever the pane is doing, this screen is not it —
/// no state, and the previous answer stands.
#[test]
fn a_transcript_scrolled_back_is_not_a_state() {
    let screen = format!(
        "● こんにちは！\n\
         \n\
         ✻ Baked for 3s · done 9:14 PM\n\
         \n\
         {BAR}\n\
         \x20 Showing detailed transcript · ctrl+o to toggle · ctrl+e to show all     verbose\n"
    );
    assert_detects(&screen, "✳ Claude Code", "nil", "transcript_viewer");
}

/// Captured: the trust dialog is a form with something to confirm, and is blocked.
#[test]
fn a_form_waiting_to_be_confirmed_is_blocked() {
    assert_detects(trust_screen(), "", "blocked", "live_blocked_form");
}

/// The `/btw` overlay is typed at a turn that is still running. Written from the manifest, and
/// half of the manifest's rule is deliberately not ported — see the rule's own comment.
#[test]
fn a_btw_overlay_is_working() {
    let screen = "/btw the port is 8080\n";
    assert_detects(screen, "", "working", "btw_overlay_working");
}

/// Captured: the spinner line of a running turn, read without the title that would outrank it.
#[test]
fn a_turn_in_progress_is_working() {
    assert_detects(&working_screen(), "", "working", "live_turn_working");
}

/// Captured: the idle screen. Two things at once — the prompt between its two rules is idle,
/// and the finished turn above it (`✻ Cogitated for 6s · done 9:01 PM`) must not read as a
/// running one. It starts with a spinner glyph and holds a `·`, and only the missing `…`
/// tells it from `* Actioning… (3s · ↓ 16 tokens)`.
#[test]
fn an_idle_prompt_is_idle_and_a_finished_turn_is_not_a_running_one() {
    assert_detects(
        &idle_screen(),
        "✳ hostname ファイルの確認",
        "idle",
        "live_prompt_box",
    );
}

/// Picking a model is a menu over the top, not a state. Written from the manifest.
#[test]
fn the_model_picker_is_not_a_state() {
    let screen = "Select Model\n1. Default\nEnter to set as default, Esc to cancel\n";
    assert_detects(screen, "", "nil", "model_picker_menu");
}

/// Captured: a Bash approval. It is `bash_permission_prompt` and not `live_blocked_form`, which
/// outranks it — the footer is "Esc to cancel · Tab to amend", with nothing to confirm.
#[test]
fn a_bash_permission_prompt_is_blocked() {
    assert_detects(
        &permission_screen(),
        "",
        "blocked",
        "bash_permission_prompt",
    );
}

/// Captured, and the one this file exists for: a permission dialog is drawn under a *single*
/// rule, so the screen has no prompt box, so `live_prompt_box` at 950 cannot claim it as idle
/// on its `❯ 1. Yes`. The dialog is claimed by the rule that knows it is a dialog.
#[test]
fn a_permission_dialog_is_not_an_idle_prompt() {
    let screen = permission_screen();
    assert!(
        !screen.contains("Enter to confirm"),
        "the capture this test is built on has changed"
    );
    assert_detects(&screen, "", "blocked", "bash_permission_prompt");
}

/// Any other approval: the question and a numbered answer, with nothing naming a command.
/// Written from the manifest — no capture of a non-Bash approval.
#[test]
fn a_generic_permission_prompt_is_blocked() {
    let screen = format!(
        "Edit src/main.rs\n\
         {BAR}\n\
         Do you want to proceed?\n\
         1. Yes\n\
         2. No\n\
         Esc to cancel\n"
    );
    assert_detects(&screen, "", "blocked", "generic_permission_prompt");
}

/// The backstop: words that read as a question left standing. Written from the manifest.
#[test]
fn a_screen_waiting_for_permission_is_blocked() {
    let screen = "Waiting for permission to continue\n";
    assert_detects(screen, "", "blocked", "legacy_no_prompt_blocker");
}

/// Captured: a Write approval, and the reason the backstop is not a leftover. It asks
/// "Do you want to create note.txt?" — the manifest's `generic_permission_prompt` gates on the
/// literal "do you want to proceed?" and so never fires here. Its diff separators are `╌`
/// (U+254C) rather than `─`, so the screen has one rule and no prompt box.
#[test]
fn a_write_approval_is_caught_by_the_backstop_and_not_by_its_own_rule() {
    let screen = format!(
        "● Write(note.txt)\n\
         \n\
         {BAR}\n\
         \x20Create file\n\
         \x20note.txt\n\
         {DASHED}\n\
         \x20 1 hello\n\
         {DASHED}\n\
         \x20Do you want to create note.txt?\n\
         \x20❯ 1. Yes\n\
         \x20  2. Yes, and switch to accept edits … for this\n\
         \x20     session (shift+tab)\n\
         \x20  3. No\n\
         \n\
         \x20Esc to cancel · Tab to amend\n"
    );
    assert!(
        !screen.contains("proceed"),
        "the capture this test is built on has changed"
    );
    assert_detects(
        &screen,
        "✳ note.txt ファイル作成",
        "blocked",
        "legacy_no_prompt_blocker",
    );
}

/// Captured: what Claude Code leaves in the title when a turn is over — a `✳` and the
/// conversation's own summary, not its name. The screen under it here is the finished turn
/// alone, with no prompt drawn yet, so the title is all there is to go on.
#[test]
fn an_idle_title_is_idle() {
    assert_detects(
        "✻ Cogitated for 6s · done 9:01 PM\n",
        "✳ hostname ファイルの確認",
        "idle",
        "osc_title_idle",
    );
}

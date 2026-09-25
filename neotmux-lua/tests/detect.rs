//! `neotmux.detect`: what a pane looks like it is doing, as a function of what is on it.
//! The rules under test here are the tests' own — the shipped Claude ruleset has its own file.
//!
//! The harness (`built`, `feed`, `keys`, `logged`, `pane`, `SIZE`) is `support`'s, shared with
//! `state.rs`: a real pane fed real bytes through `support::panes()`, with the shipped plugins
//! off so that every log line in a dispatch belongs to the test's own `on_key`.

use neotmux_proto::{Event, Size};

mod support;

use support::{
    BAR, Built, built, feed, idle_screen, keys, logged, pane, permission_screen, working_screen,
};

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

/// A closed pane is forgotten by the module itself, with no plugin that happens to listen for
/// `pane_closed` loaded: an id is reused, and the next pane given it must not inherit the dead
/// one's hold (#47).
#[test]
fn a_closed_pane_is_forgotten_without_a_plugin_to_do_it() {
    let (_dir, sub) = built(&holding(5000, None));
    feed(&sub, b"anything");
    keys(&sub, &["W"]);
    assert!(look(&sub).iter().any(|l| l == "of=working"));

    keys(&sub, &["I"]);
    assert!(look(&sub).iter().any(|l| l == "of=working"));

    sub.dispatch_event(support::one(), &Event::PaneClosed { pane: pane() });
    assert!(
        look(&sub).iter().any(|l| l == "of=idle"),
        "a pane that reused a closed pane's id inherited its hold"
    );
}

/// A ruleset whose only rule is flipped on and off by a value the test controls, so that the
/// only thing under test is what happens when *nothing* matches — as against a rule that
/// matched and declined to name a state, which `declining` below is for.
fn matching(answers: bool) -> String {
    let answers = if answers { "'idle'" } else { "nil" };
    format!(
        "local detect = require('neotmux.detect')\n\
         local says = true\n\
         detect.agents = {{\n\
           test = {{\n\
             {{ id = 'only', state = {answers}, priority = 1, visible = 'idle',\n\
               match = function() return says end }},\n\
           }},\n\
         }}\n\
         neotmux.on_key(function(key)\n\
           if key == 'Q' then says = false return true end\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(detect.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    )
}

/// Nothing matching is not "no news". A ruleset that has stopped recognising the screen has
/// nothing to say about the pane at all, and an answer it gave about an earlier screen must
/// not outlive it — otherwise a pane that has ever been claimed is never unclaimed, and
/// `neotmux.state` never reaches the patterns it falls back to.
#[test]
fn a_screen_no_rule_matches_answers_nothing_rather_than_what_it_said_last() {
    let (_dir, sub) = built(&matching(true));
    feed(&sub, b"anything");
    assert!(look(&sub).iter().any(|l| l == "of=idle"));

    keys(&sub, &["Q"]);
    assert!(
        look(&sub).iter().any(|l| l == "of=nil"),
        "a screen the ruleset no longer recognises kept its last answer"
    );
}

/// The other half: a rule that matched and answered no state — a transcript scrolled back, a
/// menu over the top — *does* leave the previous answer standing, because the pane is still
/// the ruleset's and this screen simply is not evidence.
#[test]
fn a_rule_that_matches_and_names_no_state_leaves_the_last_answer_standing() {
    let (_dir, sub) = built(&declining());
    feed(&sub, b"anything");
    assert!(look(&sub).iter().any(|l| l == "of=idle"));
    keys(&sub, &["V"]);
    assert!(
        look(&sub).iter().any(|l| l == "of=idle"),
        "a rule that matched and declined to say dropped the answer it was meant to keep"
    );
}

/// An idle rule and a viewer rule above it, flipped by a value the test controls: the viewer
/// matches and names no state, which is the one case the previous answer survives.
fn declining() -> String {
    "local detect = require('neotmux.detect')\n\
     local mode = 'idle'\n\
     detect.agents = {\n\
       test = {\n\
         { id = 'viewer', priority = 9, match = function() return mode == 'viewer' end },\n\
         { id = 'quiet', state = 'idle', priority = 1, visible = 'idle',\n\
           match = function() return mode == 'idle' end },\n\
       },\n\
     }\n\
     neotmux.on_key(function(key)\n\
       if key == 'V' then mode = 'viewer' return true end\n\
       if key == 'J' then\n\
         neotmux.log.info('of=' .. tostring(detect.of(1)))\n\
         return true\n\
       end\n\
       return false\n\
     end)\n"
        .to_string()
}

/// A ruleset is asked whether the pane is its at all before any of its rules run, and one that
/// says no is skipped whole — not one rule of it is evaluated. This is what keeps a ruleset
/// from answering for a pane whose program has quit and left a title behind.
#[test]
fn a_ruleset_that_does_not_claim_the_pane_is_never_asked() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         detect.agents = {\n\
           test = {\n\
             claims = function(info) return info.command == 'claude' end,\n\
             { id = 'only', state = 'idle', priority = 1, visible = 'idle',\n\
               match = function()\n\
                 neotmux.log.info('asked')\n\
                 return true\n\
               end },\n\
           },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(detect.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");

    support::running(&sub, "bash");
    let said = look(&sub);
    assert!(said.iter().any(|l| l == "of=nil"), "{said:?}");
    assert!(
        !said.iter().any(|l| l == "asked"),
        "a rule ran for a pane its ruleset had declined: {said:?}"
    );

    support::running(&sub, "claude");
    let said = look(&sub);
    assert!(said.iter().any(|l| l == "of=idle"), "{said:?}");
    assert!(said.iter().any(|l| l == "asked"), "{said:?}");
}

/// A `claims` that raises declines, and says so once: the ruleset is what is broken, and
/// skipping it leaves the pane where it would have been had nobody written it.
#[test]
fn a_raising_claims_declines_the_pane_and_is_reported_once() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         detect.agents = {\n\
           test = {\n\
             claims = function() error('boom') end,\n\
             { id = 'only', state = 'idle', priority = 1, match = function() return true end },\n\
           },\n\
         }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(detect.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"anything");
    let mut said = look(&sub);
    said.extend(look(&sub));
    assert_eq!(
        said.iter().filter(|l| *l == "of=nil").count(),
        2,
        "{said:?}"
    );
    assert_eq!(
        said.iter()
            .filter(|l| l.starts_with("neotmux.detect: ruleset 'test' claims:"))
            .count(),
        1,
        "{said:?}"
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

/// The lighter separators Claude Code draws around a diff. Not rules: a different character.
const DASHED: &str = "╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌";

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

/// The backstop: words that read as a question left standing. Written from the manifest, with
/// the rule Claude Code draws above anything it asks about — the backstop wants one now, because
/// the words alone are on every page of prose that talks about prompts.
#[test]
fn a_screen_waiting_for_permission_is_blocked() {
    let screen = format!("{BAR}\nWaiting for permission to continue\n");
    assert_detects(&screen, "", "blocked", "legacy_no_prompt_blocker");
}

/// And the same words with nothing drawn around them are a page of prose.
#[test]
fn the_words_without_a_dialog_around_them_are_not_a_question() {
    assert_detects("Waiting for permission to continue\n", "", "nil", "nil");
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

/// A bullet is not a spinner. `*` and `·` really are two of Claude Code's frames — the captured
/// `* Actioning… (3s · ↓ 16 tokens)` above is one — but they are also how every markdown list
/// and every `git log --graph` edge begins, and this ruleset is now asked about every pane that
/// is not a shell. `live_turn_working` answers at 970, above both permission rules, so a bullet
/// that read as working would hide a real question underneath it. What tells the two apart is
/// the clock Claude draws with the verb, which prose does not have.
#[test]
fn a_bullet_with_an_ellipsis_is_not_a_running_turn() {
    let screen = "* Installing dependencies…\n\
                  · Resolving versions…\n\
                  * Linking…\n";
    assert_detects(screen, "", "nil", "nil");
}

/// And the frame that is one, kept: the same glyph with the timer Claude Code puts after it.
#[test]
fn an_ascii_spinner_frame_with_its_timer_is_a_running_turn() {
    assert_detects(
        "* Actioning… (3s · ↓ 16 tokens)\n",
        "",
        "working",
        "live_turn_working",
    );
}

/// The backstop reads the region below the last rule and wants an answer it could pick, not the
/// word "yes" somewhere on a screen. This is this repository's own ruleset in a pager — the
/// phrase and the word are both on it, several times, and a pane showing source code is a pane
/// showing source code.
///
/// "an answer it could pick" is a shape, not a proof: `%f[%W]` is what stops `no` from matching
/// the "note…" and "nothing…" that open half the sentences in these files, and a line really
/// shaped like an answer would still pass. It narrows the rule; it does not make it sound.
#[test]
fn source_code_that_talks_about_prompts_is_not_a_prompt() {
    let screen = "      local answerable = holds(lines, \"yes\") or holds(lines, \"❯\")\n\
                  \x20     if answerable and (holds(lines, \"do you want to\")) then\n\
                  \x20       return true\n\
                  \x20     end\n\
                  \x20     return holds_any(lines, {\n\
                  \x20       \"waiting for permission\",\n\
                  \x20     })\n";
    assert_detects(screen, "", "nil", "nil");
}

/// The prose that opens with the letters of an answer. A dialog is drawn and the question is
/// asked in so many words, so everything else the backstop wants is there — and the only line
/// that could be an answer begins "Note that", which is a sentence.
#[test]
fn a_line_that_merely_starts_with_the_letters_of_an_answer_is_not_an_answer() {
    let screen = format!(
        "{BAR}\n\
         \x20Do you want to know what this rule does?\n\
         \x20Note that it reads the region below the last rule.\n\
         \x20Nothing above it draws one.\n"
    );
    assert_detects(&screen, "", "nil", "nil");
}

/// `agent` names the ruleset that answered at the last look, and nothing once none does.
#[test]
fn agent_names_the_ruleset_that_answered_the_last_look() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         detect.agents = { fake = {\n\
           { id = 'busy', state = 'working', priority = 1,\n\
             match = function(v) return v.lines[#v.lines] == 'BUSY' end },\n\
         } }\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             local before = tostring(detect.agent(1))\n\
             detect.of(1)\n\
             neotmux.log.info('before=' .. before .. ' agent=' .. tostring(detect.agent(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"BUSY");
    keys(&sub, &["J"]);
    sub.terminals().feed(pane(), b"\x1b[2J\x1b[Hsomething else");
    keys(&sub, &["J"]);
    let said: Vec<String> = logged(&sub)
        .into_iter()
        .filter(|l| l.starts_with("before="))
        .collect();
    assert_eq!(
        said,
        vec!["before=nil agent=fake", "before=fake agent=nil"],
        "{said:?}"
    );
}

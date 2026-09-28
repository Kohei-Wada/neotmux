//! `neotmux.state`: `M.of` asks `neotmux.detect` first and falls back to its own pattern list
//! only for a pane no ruleset claims.
//!
//! The harness (`built`, `feed`, `keys`, `logged`, `pane`, `SIZE`) is `support`'s, shared with
//! `detect.rs`: a real pane fed real bytes through `support::panes()`, with the shipped plugins
//! off so that every log line in a dispatch belongs to the test's own `on_key`.

mod support;

use support::{built, feed, keys, logged, pane, running, running_unreadable};

/// A Claude screen gets the detector's answer, not the fallback's clock. The first look
/// establishes both: the detector's own cache, which persists a `working` verdict across looks
/// at the same revision regardless of wall time, and the fallback's, which times out. Reading a
/// second time after `working_for` has elapsed — with the screen unchanged, so neither cache
/// rebuilds — is what tells the two apart: a fallback with a one-millisecond window would have
/// gone stale, and only a detector still reading this screen as working can still be saying so.
/// (Verified against a stub that makes `M.of` skip the detector: it fails, `of=idle`, exactly
/// as it should — see task-5-report.md.)
#[test]
fn a_pane_a_ruleset_claims_gets_the_detectors_answer() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         state.working_for = 1\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(
        &sub,
        "\u{273b} Herding… (12s \u{00b7} esc to interrupt)".as_bytes(),
    );
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=working"),
        "the detector was not asked"
    );

    std::thread::sleep(std::time::Duration::from_millis(20));
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=working"),
        "the detector's answer did not survive past working_for — the pattern list would call \
         this idle, and the fallback's window has long since closed"
    );
}

/// A pane no ruleset claims still gets the old guess: `[y/N]` is a question, and nothing in the
/// Claude ruleset has anything to say about a shell. The detector genuinely answers nil for this
/// screen, so this one exercises the fallback whether or not `M.of` asks the detector first —
/// its job is to prove the fallback still works, not to prove it was reached instead of the
/// detector; the first test covers that half.
#[test]
fn a_pane_no_ruleset_claims_falls_back_to_the_patterns() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"rm -i file\r\nremove file? [y/N] ");
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=blocked"),
        "the fallback stopped working"
    );
}

/// And the fallback's "anything that printed just now is working" still holds, in milliseconds
/// against the new clock rather than seconds against `os.time`. Plain shell output, so the
/// detector answers nil here too and this one is testing the fallback's own clock, not which
/// branch it took.
#[test]
fn the_fallback_still_calls_a_pane_that_just_printed_working() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         state.working_for = 5000\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, b"building something");
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=working"),
        "a pane that just printed is working"
    );
}

/// The whole point of `claims`, end to end against the shipped ruleset. Claude Code writes a
/// `✳` into the terminal title when a turn is over, and nothing resets it when the program
/// exits — `neotmux-pty` keeps the last title a pane set forever. So the shell you go on using
/// in that pane still carries it, and `osc_title_idle` would answer `idle` for every screen
/// thereafter: no `blocked` for an `rm -i`, no tint, no chime, in the most ordinary life a
/// Claude pane has.
///
/// What tells the two apart is what the pane is running, which the host reads out of `/proc`:
/// `claude` while it is up, the shell's own name the moment it is quit.
#[test]
fn a_pane_whose_agent_has_quit_stops_being_answered_for_by_what_it_left_behind() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, "\x1b]2;\u{2733} hostname の確認\x07".as_bytes());

    running(&sub, "claude");
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=idle"),
        "the title rule stopped answering for a pane that is running Claude Code"
    );

    // Claude is quit; the title it left behind is still the pane's, so this writes into the
    // same terminal rather than opening a fresh one — `open` would wipe the title with it.
    running(&sub, "bash");
    sub.terminals()
        .feed(pane(), b"\r\nrm -i file\r\nremove file? [y/N] ");
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=blocked"),
        "a shell asking a question read as an idle agent, off a title nobody reset"
    );
}

/// And the other half: a pane running Claude Code through a wrapper is still Claude Code's.
/// A shell function that puts `systemd-inhibit` (or `env`, `nohup`, `sudo`, `npx`) in front of
/// the binary leaves the foreground process group leader named after the wrapper, so an
/// allow-list of `claude` and `node` declines the pane and the whole detector is dead for that
/// user. The ruleset names the shells instead, and this screen — a live turn, which
/// `live_turn_working` reads — has to come back `working` rather than falling through to the
/// pattern list.
#[test]
fn a_pane_running_the_agent_under_a_wrapper_is_still_claimed() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         state.working_for = 1\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    running(&sub, "systemd-inhibit");
    feed(
        &sub,
        "\u{273b} Herding… (12s \u{00b7} esc to interrupt)".as_bytes(),
    );
    // Past `working_for`, so the fallback would answer `idle` here and only the detector can
    // still be saying `working` — the same trick as the first test in this file.
    std::thread::sleep(std::time::Duration::from_millis(20));
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=working"),
        "a wrapped Claude Code was declined, and with it the whole detector"
    );
}

/// The other end of the same thread: claiming by default means a pane that ran Claude Code and
/// then ran something that is not a shell — `less`, `python`, a script — is claimed too, and it
/// is still wearing the `✳ ` title Claude left in the terminal. Nothing resets a title, so
/// `osc_title_idle` would answer `idle` for that pane until the session ended: a pager reading
/// done, and the question underneath it never heard.
///
/// So a title counts only while the program that set it is the program that is running.
/// `neotmux.detect` blanks it when the pane's command has changed since the title was last
/// seen, which puts this pane back where it belongs — the pattern fallback, and `[y/N]`.
#[test]
fn a_pane_that_ran_an_agent_and_now_runs_a_pager_is_not_answered_by_the_title_it_kept() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    feed(&sub, "\x1b]2;\u{2733} hostname の確認\x07".as_bytes());

    running(&sub, "claude");
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=idle"),
        "the title rule stopped answering for a pane that is running Claude Code"
    );

    // Claude is gone and a pager has the pane. The title is untouched — a terminal keeps it —
    // so this writes into the same terminal rather than opening a fresh one.
    running(&sub, "less");
    sub.terminals()
        .feed(pane(), b"\r\nOverwrite existing file? [y/N] ");
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=blocked"),
        "a pager asking a question read as an idle agent, off a title nobody reset"
    );
}

/// The cost of §3.2b's nil rule, pinned so that changing it is a decision and not an accident.
///
/// A title whose first sighting could not be matched to a program has no owner on record, and
/// `neotmux.detect` never gives it one. Four looks: the title arrives while `/proc` says
/// nothing, then a pager has the pane, then something else does, then that something else asks
/// a question. The title is believed through all four — including the last, where a real
/// `blocked` is on the screen and goes unheard.
///
/// This is the looser of the two behaviours, measurably: writing the next readable command down
/// as the owner instead gives `idle / idle / working / blocked` for these same four looks —
/// ownership comes off at the third command and the question is heard. It is chosen anyway, and
/// the reason is the platform rather than which guess is nicer. `pane.info().command` is
/// `/proc`: off Linux it is *always* nil, so a rule that distrusted a title it could not match
/// to a program would blank every title on every non-Linux host and take both title rules with
/// it. Trust is wrong on one path; distrust is wrong on one platform.
#[test]
fn a_title_whose_owner_was_never_readable_is_believed_until_something_writes_another() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         local n = 0\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             n = n + 1\n\
             neotmux.log.info('look' .. n .. '=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );

    // The title arrives while nothing can be read about the pane — the `/proc` miss that
    // happens when a process is going away, which is when an agent's title is left behind.
    feed(&sub, "\x1b]2;\u{2733} hostname の確認\x07".as_bytes());
    running_unreadable(&sub);
    keys(&sub, &["J"]);

    // A pager, then something else. Each look needs a byte to move the revision, or the
    // detector answers out of the look it already took.
    running(&sub, "less");
    sub.terminals().feed(pane(), b"\r\n:");
    keys(&sub, &["J"]);

    running(&sub, "python3");
    sub.terminals().feed(pane(), b"\r\n>>> ");
    keys(&sub, &["J"]);

    sub.terminals().feed(pane(), b"\r\nContinue? [y/N] ");
    keys(&sub, &["J"]);

    assert_eq!(
        logged(&sub),
        vec![
            "look1=idle".to_string(),
            "look2=idle".to_string(),
            "look3=idle".to_string(),
            "look4=idle".to_string(),
        ],
        "the title with no owner on record is believed through every program that follows, and \
         the question at the fourth look is the price of that"
    );
}

/// A program that changes without printing is noticed all the same. The look is cached against
/// the screen's revision, so a pane whose agent is quit — back at a shell that has drawn nothing
/// yet — used to go on being answered for by the agent's ruleset until the shell next printed.
#[test]
fn a_pane_whose_agent_quit_without_a_new_screen_is_asked_again() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             state.of(1)\n\
             neotmux.log.info('agent=' .. tostring(detect.agent(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    running(&sub, "claude");
    feed(
        &sub,
        "\u{273b} Herding… (12s \u{00b7} esc to interrupt)".as_bytes(),
    );
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub).last().map(String::as_str),
        Some("agent=claude"),
        "the turn was not Claude Code's in the first place"
    );

    running(&sub, "bash");
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub).last().map(String::as_str),
        Some("agent=nil"),
        "the quit agent's ruleset still answered for a screen nobody redrew"
    );
}

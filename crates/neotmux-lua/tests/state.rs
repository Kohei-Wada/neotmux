//! `neotmux.state`: `M.of` asks `neotmux.detect` first and falls back to its own pattern list
//! only for a pane no ruleset claims.
//!
//! The harness (`built`, `feed`, `keys`, `logged`, `pane`, `SIZE`) is `support`'s, shared with
//! `detect.rs`: a real pane fed real bytes through `support::panes()`, with the shipped plugins
//! off so that every log line in a dispatch belongs to the test's own `on_key`.

mod support;

use support::{built, feed, keys, logged};

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

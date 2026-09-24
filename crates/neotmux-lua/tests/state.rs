//! `neotmux.state`: `M.of` asks `neotmux.detect` first and falls back to its own pattern list
//! only for a pane no ruleset claims.
//!
//! The harness is `detect.rs`'s: a real pane fed real bytes through `support::panes()`, with the
//! shipped plugins off so that every log line in a dispatch belongs to the test's own `on_key`.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
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
            neotmux_lua::KeyOutcome::Consumed,
            "the test's own on_key did not take '{spec}'"
        );
    }
}

fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A Claude screen gets the detector's answer. The pattern list would call this idle — there
/// is no question mark on it — and the detector calls it working, which is the point.
#[test]
fn a_pane_a_ruleset_claims_gets_the_detectors_answer() {
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
    feed(
        &sub,
        "\u{273b} Herding… (12s \u{00b7} esc to interrupt)".as_bytes(),
    );
    keys(&sub, &["J"]);
    assert!(
        logged(&sub).iter().any(|l| l == "of=working"),
        "the detector was not asked"
    );
}

/// A pane no ruleset claims still gets the old guess: `[y/N]` is a question, and nothing in the
/// Claude ruleset has anything to say about a shell.
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
/// against the new clock rather than seconds against `os.time`.
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

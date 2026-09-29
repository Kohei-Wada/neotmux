//! `plugin.tmux.borders` beside panes that have given columns to a pane band.

mod support;

use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect};
use support::{Built, built, one};

/// Two panes, one above the other, over the whole 40x16 surface with a one-row gutter between
/// them, each with `band` columns given up on its left.
fn stacked(sub: &Built, band: u16) {
    stacked_at(sub, band, 7);
}

/// The same, with the gutter at row `rule`.
fn stacked_at(sub: &Built, band: u16, rule: u16) {
    let (top, bottom) = (PaneId::from(1), PaneId::from(2));
    let top_outer = Rect::new(0, 0, 40, rule);
    let bottom_outer = Rect::new(0, rule + 1, 40, 16 - rule - 1);
    let inset = |r: Rect| Rect::new(r.x + band, r.y, r.w - band, r.h);
    sub.begin_frame(
        one(),
        Epoch::new(support::SIZE, vec![top, bottom], Some(top)),
        Geometry {
            area: Rect::new(0, 0, 40, 16),
            panes: vec![(top, inset(top_outer)), (bottom, inset(bottom_outer))],
            outer: vec![(top, top_outer), (bottom, bottom_outer)],
            focus: Some(top),
            ..Default::default()
        },
    );
    sub.dispatch_tick(one());
}

#[test]
fn the_rule_under_a_banded_pane_runs_under_its_band_too() {
    let (_dir, sub) = built("require('plugin.tmux.borders')");
    stacked(&sub, 10);
    let starts: Vec<u16> = sub
        .frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Line {
                from: (x, 7),
                to: (_, 7),
                ..
            } => Some(x),
            _ => None,
        })
        .collect();
    assert_eq!(
        starts.iter().min(),
        Some(&0),
        "the rule starts at the band, not at the content: {starts:?}"
    );
}

/// The dividers are a function of where the panes are, so a tick with the panes where they were
/// works nothing out again, and one after they moved does.
#[test]
fn the_crossings_are_worked_out_again_only_when_the_panes_move() {
    let (_dir, sub) = built(
        "require('plugin.tmux.borders')\n\
         local junctions = require('neotmux.junctions')\n\
         local of = junctions.of\n\
         junctions.of = function(...) neotmux.log.info('worked out') return of(...) end\n",
    );
    stacked(&sub, 10);
    stacked(&sub, 10);
    stacked(&sub, 10);
    assert_eq!(support::logged(&sub), vec!["worked out"]);
    // A band is inside a pane's outer rect, so resizing one moves no divider.
    stacked(&sub, 0);
    assert!(support::logged(&sub).is_empty());
    stacked_at(&sub, 0, 9);
    assert_eq!(support::logged(&sub), vec!["worked out"]);
}

/// Drawn every tick all the same: what a tick draws is the whole of its output.
#[test]
fn a_tick_with_the_panes_where_they_were_still_draws_the_rule() {
    let (_dir, sub) = built("require('plugin.tmux.borders')");
    stacked(&sub, 0);
    stacked(&sub, 0);
    let rules = sub
        .frame_output()
        .into_iter()
        .filter(|cmd| matches!(cmd, DrawCmd::Line { from: (_, 7), .. }))
        .count();
    assert_eq!(rules, 1);
}

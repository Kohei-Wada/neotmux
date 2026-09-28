//! `plugin.tmux.borders` beside panes that have given columns to a pane band.

mod support;

use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect};
use support::{Built, built, one};

/// Two panes, one above the other, over the whole 40x16 surface with a one-row gutter between
/// them, each with `band` columns given up on its left.
fn stacked(sub: &Built, band: u16) {
    let (top, bottom) = (PaneId::from(1), PaneId::from(2));
    let top_outer = Rect::new(0, 0, 40, 7);
    let bottom_outer = Rect::new(0, 8, 40, 8);
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

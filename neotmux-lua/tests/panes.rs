use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::geom::Window;
use neotmux_proto::{Dir, Epoch, Geometry, PaneId, Rect, SessionId, Size, WindowId};
use neotmux_proto::{LineMode, PaneTty};

mod support;

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

/// Two side-by-side panes on an 80x24 surface, the second focused.
fn publish(sub: &LuaSubsystem, session: SessionId, panes: Vec<(PaneId, Rect)>, focus: PaneId) {
    let size = Size { rows: 24, cols: 80 };
    sub.begin_frame(
        session,
        Epoch::new(size, panes.iter().map(|(id, _)| *id).collect(), Some(focus)),
        Geometry {
            area: Rect::new(0, 0, size.cols, size.rows),
            windows: vec![Window {
                id: WindowId::from(1),
                panes: panes.iter().map(|(id, _)| *id).collect(),
                name: None,
            }],
            window: Some(WindowId::from(1)),
            panes,
            focus: Some(focus),
        },
    );
}

fn side_by_side(sub: &LuaSubsystem) {
    publish(
        sub,
        support::one(),
        vec![
            (PaneId::from(1), Rect::new(0, 0, 40, 24)),
            (PaneId::from(2), Rect::new(40, 0, 40, 24)),
        ],
        PaneId::from(2),
    );
}

fn logged(sub: &LuaSubsystem, session: Option<SessionId>) -> Vec<String> {
    logged_from(&sub.drain_ops(session))
}

/// The same reading, off ops already drained: a test that has to look at what else was raised
/// cannot drain twice.
fn logged_from<S>(ops: &[(S, Op)]) -> Vec<String> {
    ops.iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

const SPELL: &str = r#"
local function spell(prefix, r)
  if not r then
    neotmux.log.info(prefix .. ' nil')
    return
  end
  neotmux.log.info(prefix .. ' ' .. r.x .. ',' .. r.y .. ',' .. r.w .. ',' .. r.h)
end
"#;

/// The asymmetry this closes: the rects were reachable only from inside a tick, so a plugin
/// that wanted geometry had to register a draw callback it did not want.
#[test]
fn a_pane_rect_read_from_a_key_handler_matches_the_one_read_from_a_tick() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.asker')"),
        (
            "lua/plugin/asker.lua",
            &format!(
                "{SPELL}
                neotmux.on_key(function() spell('key', neotmux.pane.rect(2)) return true end)
                neotmux.on_tick(function(ctx) spell('tick', ctx:pane_rect(2)) end)
                return {{}}"
            ),
        ),
    ]);
    side_by_side(&sub);
    sub.dispatch_key(support::one(), "C-x");
    sub.dispatch_tick(support::one());

    let lines = logged(&sub, Some(support::one()));
    assert_eq!(
        lines,
        vec!["key 40,0,40,24", "tick 40,0,40,24"],
        "{lines:?}"
    );
}

#[test]
fn a_pane_nobody_has_has_no_rect() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.asker')"),
        (
            "lua/plugin/asker.lua",
            &format!(
                "{SPELL}
                neotmux.on_key(function() spell('key', neotmux.pane.rect(99)) return true end)
                return {{}}"
            ),
        ),
    ]);
    side_by_side(&sub);
    sub.dispatch_key(support::one(), "C-x");
    assert_eq!(logged(&sub, Some(support::one())), vec!["key nil"]);
}

/// A resize is an op like a split: queued from wherever it was asked for, applied after the
/// tick, and carrying the direction and the cells the caller named.
#[test]
fn a_resize_is_queued_as_an_op_with_the_direction_and_the_cells() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.grower')"),
        (
            "lua/plugin/grower.lua",
            "neotmux.on_key(function() neotmux.pane.resize('left', 3) return true end)
             return {}",
        ),
    ]);
    side_by_side(&sub);
    sub.dispatch_key(support::one(), "C-x");
    let ops: Vec<Op> = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .map(|(_, op)| op)
        .collect();
    assert_eq!(
        ops,
        vec![Op::PaneResize {
            dir: Dir::Left,
            cells: 3
        }]
    );
}

/// A direction nobody can spell and a cell count that is not one are typos, so they raise —
/// the shape `pane.split` and `keymap.set` already use for a malformed argument. A refusal
/// cannot be returned here at all: the op is applied long after the call returns.
#[test]
fn a_malformed_resize_raises_and_queues_nothing() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.grower')"),
        (
            "lua/plugin/grower.lua",
            "local function try(...)
               local ok, err = pcall(neotmux.pane.resize, ...)
               neotmux.log.info(tostring(ok) .. ' ' .. tostring(err))
             end
             neotmux.on_key(function() try('sideways', 1) try('left', 0) try('left', -2) return true end)
             return {}",
        ),
    ]);
    side_by_side(&sub);
    sub.dispatch_key(support::one(), "C-x");
    let lines = logged(&sub, Some(support::one()));
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].starts_with("false") && lines[0].contains("unknown direction 'sideways'"),
        "{lines:?}"
    );
    assert!(
        lines[1].contains("positive number of cells, got 0"),
        "{lines:?}"
    );
    assert!(
        lines[2].contains("positive number of cells, got -2"),
        "{lines:?}"
    );
}

/// The runtime layer neotmux ships, read straight out of the source tree: the shipped prefix
/// is the plugin under test here, not a fixture standing in for it.
fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

fn with_prefix() -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), "require('plugin.prefix')").unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn ops(sub: &LuaSubsystem) -> Vec<Op> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .map(|(_, op)| op)
        .collect()
}

/// A tall pane on the left, and two of unequal height stacked on the right. Pressing `l`
/// from the left one has two candidates touching its edge, and the answer is the one facing
/// most of it — not the first in the list, which is the four-row sliver at the top.
#[test]
fn directional_focus_picks_the_pane_facing_most_of_the_edge() {
    let (_d, sub) = with_prefix();
    publish(
        &sub,
        support::one(),
        vec![
            (PaneId::from(1), Rect::new(0, 0, 40, 24)),
            (PaneId::from(2), Rect::new(41, 0, 39, 4)),
            (PaneId::from(3), Rect::new(41, 5, 39, 19)),
        ],
        PaneId::from(1),
    );

    sub.dispatch_key(support::one(), "C-b");
    sub.dispatch_key(support::one(), "l");
    assert_eq!(
        ops(&sub),
        vec![Op::PaneFocus {
            pane: PaneId::from(3)
        }],
        "the pane sharing 19 rows with the focused one, not the 4-row one listed first"
    );
}

/// The window has an edge and the focus stops at it: no wrapping round to the far side.
#[test]
fn there_is_nothing_past_the_edge_of_the_window() {
    let (_d, sub) = with_prefix();
    side_by_side(&sub);
    // Pane 2 is the right-hand one: nothing is to its right, above it or below it.
    for key in ["l", "j", "k"] {
        sub.dispatch_key(support::one(), "C-b");
        assert_eq!(sub.dispatch_key(support::one(), key), KeyOutcome::Consumed);
        assert!(
            ops(&sub).is_empty(),
            "{key} from the right-hand pane focused something"
        );
    }
    sub.dispatch_key(support::one(), "C-b");
    sub.dispatch_key(support::one(), "h");
    assert_eq!(
        ops(&sub),
        vec![Op::PaneFocus {
            pane: PaneId::from(1)
        }]
    );
}

/// Resize mode is a flag in the plugin and an `on_key`, like the prefix itself: the host
/// learns nothing about modes. Every key in it is swallowed, and the first that is not an
/// adjustment leaves.
#[test]
fn resize_mode_adjusts_until_a_key_that_is_not_an_adjustment() {
    let (_d, sub) = with_prefix();
    side_by_side(&sub);

    sub.dispatch_key(support::one(), "C-b");
    sub.dispatch_key(support::one(), "r");
    assert!(ops(&sub).is_empty(), "entering the mode is not an op");

    for key in ["h", "h", "k"] {
        assert_eq!(sub.dispatch_key(support::one(), key), KeyOutcome::Consumed);
    }
    assert_eq!(
        ops(&sub),
        vec![
            Op::PaneResize {
                dir: Dir::Left,
                cells: 1
            },
            Op::PaneResize {
                dir: Dir::Left,
                cells: 1
            },
            Op::PaneResize {
                dir: Dir::Up,
                cells: 1
            },
        ]
    );

    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "x"),
        KeyOutcome::Fallthrough,
        "the mode outlived the key that left it"
    );
    assert!(ops(&sub).is_empty());
}

/// One Lua layer, two sessions: what a tick reads is the geometry of the session whose tick
/// it is, and what it raises belongs to that session. Nothing in the program makes a second
/// session yet — the layer is where the shape is proved first.
#[test]
fn a_tick_reads_and_raises_against_the_session_it_is_for() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function() neotmux.log.info('panes ' .. #neotmux.pane.list()) end)",
    )]);
    let one = support::one();
    let two = SessionId::from(2);
    let wide = vec![
        (PaneId::from(1), Rect::new(0, 0, 40, 24)),
        (PaneId::from(2), Rect::new(40, 0, 40, 24)),
    ];
    let lone = vec![(PaneId::from(3), Rect::new(0, 0, 80, 24))];

    publish(&sub, one, wide.clone(), PaneId::from(2));
    sub.dispatch_tick(one);
    publish(&sub, two, lone, PaneId::from(3));
    sub.dispatch_tick(two);
    publish(&sub, one, wide, PaneId::from(2));
    sub.dispatch_tick(one);

    assert_eq!(
        logged(&sub, Some(one)),
        vec!["panes 2".to_string(), "panes 2".to_string()],
        "session one's ticks saw session two's geometry, or its ops went to the wrong session"
    );
    assert_eq!(logged(&sub, Some(two)), vec!["panes 1".to_string()]);
}

/// A load happens before any session does. What it reads is an empty geometry rather than
/// some session's — there is not one yet — and what it raises belongs to no session, which is
/// the case `drain_ops(None)` exists for and the one `Plugins::drain_load_ops` asks.
#[test]
fn a_config_load_runs_with_no_session_bound() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.log.info('panes at load ' .. #neotmux.pane.list())",
    )]);

    assert_eq!(
        logged(&sub, None),
        vec!["panes at load 0".to_string()],
        "a load was handed a session's geometry, or its ops were filed under one"
    );
    assert!(
        logged(&sub, Some(support::one())).is_empty(),
        "an op raised before any session existed was filed under a session"
    );
}

/// `pane.send` is the in-process half of `neotmux send`: text into a pane's shell, queued as
/// an op like every other request a tick makes.
#[test]
fn a_send_is_an_op_carrying_the_text_as_written() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function() neotmux.pane.send(1, 'echo hi\\n') end)",
    )]);
    side_by_side(&sub);
    sub.dispatch_tick(support::one());
    assert_eq!(
        sub.drain_ops(Some(support::one()))
            .into_iter()
            .map(|(_, op)| op)
            .collect::<Vec<_>>(),
        vec![Op::PaneSend {
            pane: PaneId::from(1),
            text: "echo hi\n".to_string(),
        }]
    );
}

/// The one thing that can be answered at the moment of the call: whether the session holds
/// that pane. Nothing is queued for a pane that is not there.
#[test]
fn a_send_to_a_pane_the_session_does_not_hold_answers_false_and_queues_nothing() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function()
           local ok, why = neotmux.pane.send(99, 'nothing')
           neotmux.log.info(tostring(ok) .. ' ' .. tostring(why))
         end)",
    )]);
    side_by_side(&sub);
    sub.dispatch_tick(support::one());
    let ops = sub.drain_ops(Some(support::one()));
    assert_eq!(logged_from(&ops), vec!["false no pane %99".to_string()]);
    assert!(
        !ops.iter().any(|(_, op)| matches!(op, Op::PaneSend { .. })),
        "a send was queued for a pane that is not there"
    );
}

/// A pane in a window nobody is looking at is exactly the pane a plugin answers, so it is
/// reachable: `windows`, not this frame's `panes`, is what a send is checked against.
#[test]
fn a_send_reaches_a_pane_in_a_window_that_is_not_shown() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function() neotmux.pane.send(7, 'hidden\\n') end)",
    )]);
    let size = Size { rows: 24, cols: 80 };
    let shown = PaneId::from(1);
    sub.begin_frame(
        support::one(),
        Epoch::new(size, vec![shown], Some(shown)),
        Geometry {
            area: Rect::new(0, 0, size.cols, size.rows),
            panes: vec![(shown, Rect::new(0, 0, 80, 24))],
            focus: Some(shown),
            windows: vec![
                Window {
                    id: WindowId::from(1),
                    panes: vec![shown],
                    name: None,
                },
                Window {
                    id: WindowId::from(2),
                    panes: vec![PaneId::from(7)],
                    name: None,
                },
            ],
            window: Some(WindowId::from(1)),
        },
    );
    sub.dispatch_tick(support::one());
    assert!(
        sub.drain_ops(Some(support::one()))
            .into_iter()
            .any(|(_, op)| matches!(op, Op::PaneSend { pane, .. } if pane == PaneId::from(7))),
        "a pane in a hidden window was refused"
    );
}

/// A tick that overruns loses its frame, and the send goes with it: nothing is written during
/// a tick, so there is no half of this to take back.
#[test]
fn a_send_from_a_tick_that_raises_is_discarded_with_the_rest_of_its_frame() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function()
           neotmux.pane.send(1, 'never arrives')
           error('and then it raised')
         end)",
    )]);
    side_by_side(&sub);
    sub.dispatch_tick(support::one());
    assert!(
        !sub.drain_ops(Some(support::one()))
            .iter()
            .any(|(_, op)| matches!(op, Op::PaneSend { .. })),
        "a raising tick's send survived it"
    );
}

/// Where the cursor is and what the program has done to the terminal: published with the
/// frame, read from a tick like anything else about a pane.
#[test]
fn the_cursor_and_the_mode_are_this_frame_s() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function()
           local at = neotmux.pane.cursor(1)
           local mode = neotmux.pane.mode(1)
           neotmux.log.info(at.x .. ',' .. at.y .. ' visible=' .. tostring(at.visible)
             .. ' canonical=' .. tostring(mode.canonical)
             .. ' echo=' .. tostring(mode.echo)
             .. ' alternate=' .. tostring(mode.alternate))
         end)",
    )]);
    side_by_side(&sub);
    sub.publish_pane_tty(
        support::one(),
        BTreeMap::from([(
            PaneId::from(1),
            PaneTty {
                cursor: (4, 12),
                cursor_visible: true,
                alternate: false,
                application_cursor: false,
                line: Some(LineMode {
                    canonical: true,
                    echo: true,
                }),
            },
        )]),
    );
    sub.dispatch_tick(support::one());
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["12,4 visible=true canonical=true echo=true alternate=false".to_string()]
    );
}

/// A pane whose master would not answer is a pane whose line discipline is *unknown*, and
/// unknown is absent rather than false: a plugin that cannot tell a password prompt from a
/// shell must do nothing rather than something.
#[test]
fn a_mode_the_host_could_not_read_leaves_the_flags_out_rather_than_guessing() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function()
           local mode = neotmux.pane.mode(1)
           neotmux.log.info('canonical=' .. tostring(mode.canonical)
             .. ' alternate=' .. tostring(mode.alternate))
         end)",
    )]);
    side_by_side(&sub);
    sub.publish_pane_tty(
        support::one(),
        BTreeMap::from([(
            PaneId::from(1),
            PaneTty {
                alternate: true,
                ..PaneTty::default()
            },
        )]),
    );
    sub.dispatch_tick(support::one());
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["canonical=nil alternate=true".to_string()]
    );
}

/// A pane the frame said nothing about — one that has just closed — answers nil rather than
/// a table of defaults that reads as fact.
#[test]
fn a_pane_the_frame_did_not_describe_has_no_cursor_and_no_mode() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_tick(function()
           neotmux.log.info(tostring(neotmux.pane.cursor(1)) .. ' ' .. tostring(neotmux.pane.mode(1)))
         end)",
    )]);
    side_by_side(&sub);
    sub.dispatch_tick(support::one());
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["nil nil".to_string()]
    );
}

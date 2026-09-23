use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{Dir, Epoch, Geometry, PaneId, Rect, SessionId, Size};

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
            size,
            panes,
            focus: Some(focus),
            ..Default::default()
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
    sub.drain_ops(session)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
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

    for key in ["Left", "h", "Up"] {
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

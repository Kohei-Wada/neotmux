use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{LuaSubsystem, WriteVerdict};
use neotmux_proto::geometry::Window;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, SessionId, Size, WindowId};

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

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// The shipped runtime on the path behind the config, so `require('neotmux.guard')` resolves
/// the way it would for a real user's `init.lua`.
fn shipped(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
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
    sub.drain_ops(session)
        .iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The handler sees the write exactly as it was asked for, and passing it is the default:
/// a handler that answers nothing has no opinion.
#[test]
fn a_handler_that_says_nothing_passes_the_write() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_write(function(write)
           neotmux.log.info(write.pane .. ' ' .. write.text .. ' ' .. write.from)
         end)",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "ls -l\n", "tool"),
        WriteVerdict::Pass
    );
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["1 ls -l\n tool".to_string()]
    );
}

#[test]
fn a_handler_that_refuses_stops_the_write_and_its_reason_is_the_answer() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_write(function() return false, 'not on my watch' end)",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "rm -rf /", "tool"),
        WriteVerdict::Refused("not on my watch".to_string())
    );
}

/// A refusal is a decision, not a vote: nothing after it runs.
#[test]
fn the_handlers_after_a_refusal_are_not_called() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        (
            "lua/plugin/a.lua",
            "neotmux.on_write(function() return false, 'first' end) return {}",
        ),
        (
            "lua/plugin/b.lua",
            "neotmux.on_write(function() neotmux.log.info('b ran') end) return {}",
        ),
    ]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "x", "tool"),
        WriteVerdict::Refused("first".to_string())
    );
    assert!(
        logged(&sub, Some(support::one())).is_empty(),
        "a handler past the refusal ran"
    );
}

/// The rule this whole thing rests on: a guardrail that fails open is useless exactly when
/// something is wrong.
#[test]
fn a_handler_that_raises_refuses_the_write() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.on_write(function() error('the guard itself is broken') end)",
    )]);
    side_by_side(&sub);
    let WriteVerdict::Refused(why) =
        sub.dispatch_write(support::one(), PaneId::from(1), "x", "tool")
    else {
        panic!("a raising handler passed the write");
    };
    assert!(why.contains("init"), "{why}");
}

#[test]
fn a_handler_that_overruns_its_budget_refuses_the_write() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.opt.tick_budget_ms = 5
         neotmux.on_write(function() while true do end end)",
    )]);
    side_by_side(&sub);
    assert!(matches!(
        sub.dispatch_write(support::one(), PaneId::from(1), "x", "tool"),
        WriteVerdict::Refused(_)
    ));
}

/// A handler earlier in the snapshot can remove a later one's registration before the
/// dispatch reaches it — the same mutation `registry.rs`'s
/// `snapshot_is_stable_while_a_callback_mutates_the_registry` pins from the registry's own
/// side. From `offer_write`'s side the removed id's callback and owner are simply gone, and
/// that is "cannot be reached", which refuses rather than silently passing.
#[test]
fn a_handler_removed_from_under_the_dispatch_refuses_rather_than_passing() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "local b
         neotmux.on_write(function() b:remove() end)
         b = neotmux.on_write(function() neotmux.log.info('b ran') end)",
    )]);
    side_by_side(&sub);
    let verdict = sub.dispatch_write(support::one(), PaneId::from(1), "x", "tool");
    assert!(
        matches!(verdict, WriteVerdict::Refused(_)),
        "a handler pulled out from under the dispatch passed the write: {verdict:?}"
    );
    assert!(
        logged(&sub, Some(support::one())).is_empty(),
        "the removed handler ran anyway"
    );
}

/// The cheap question the door asks before it decides which path a `send` takes.
#[test]
fn nothing_registered_is_nothing_listening() {
    let (_dir, sub) = built(&[("init.lua", "neotmux.on_tick(function() end)")]);
    assert!(!sub.has_write_hooks());
    let (_dir, sub) = built(&[("init.lua", "neotmux.on_write(function() end)")]);
    assert!(sub.has_write_hooks());
}

/// The convention on top of the hook: a pattern table, and one handler for all of it.
#[test]
fn the_guard_convention_refuses_a_line_that_matches_a_pattern() {
    let (_dir, sub) = shipped(&[(
        "init.lua",
        "local guard = require('neotmux.guard')
         guard.block('rm%s+%-%w*[rf]', 'rm -rf is blocked')",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "rm -rf /tmp/x\n", "tool"),
        WriteVerdict::Refused("rm -rf is blocked".to_string())
    );
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "ls -l\n", "tool"),
        WriteVerdict::Pass
    );
}

/// `only` is what makes a rule an agent's rule rather than everyone's.
#[test]
fn a_guard_can_apply_to_one_kind_of_writer() {
    let (_dir, sub) = shipped(&[(
        "init.lua",
        "local guard = require('neotmux.guard')
         guard.only('tool')
         guard.block('careful', 'not from a tool')",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "careful\n", "tool"),
        WriteVerdict::Refused("not from a tool".to_string())
    );
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "careful\n", "plugin"),
        WriteVerdict::Pass
    );
}

/// `plain` is what makes the obvious rule work: a literal with `-`, `.` and `%` in it blocks
/// exactly the text it names, and nothing that only looks like it.
#[test]
fn plain_blocks_the_literal_and_not_a_near_miss() {
    let (_dir, sub) = shipped(&[(
        "init.lua",
        "local guard = require('neotmux.guard')
         guard.block(guard.plain('git push --force%'), 'no force pushes')",
    )]);
    side_by_side(&sub);
    assert_eq!(
        sub.dispatch_write(
            support::one(),
            PaneId::from(1),
            "git push --force%\n",
            "tool"
        ),
        WriteVerdict::Refused("no force pushes".to_string())
    );
    assert_eq!(
        sub.dispatch_write(support::one(), PaneId::from(1), "git pushforce\n", "tool"),
        WriteVerdict::Pass
    );
}

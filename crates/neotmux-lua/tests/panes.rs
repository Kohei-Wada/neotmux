use std::fs;

use neotmux_core::proto::{Dir, Epoch, PaneId, Rect, Size};
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{Geometry, LuaSubsystem};

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

/// Two side-by-side panes on an 80x24 surface, the second focused.
fn publish(sub: &LuaSubsystem, panes: Vec<(PaneId, Rect)>, focus: PaneId) {
    let size = Size { rows: 24, cols: 80 };
    sub.begin_frame(
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
        vec![
            (PaneId::from(1), Rect::new(0, 0, 40, 24)),
            (PaneId::from(2), Rect::new(40, 0, 40, 24)),
        ],
        PaneId::from(2),
    );
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops()
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
    sub.dispatch_key("C-x");
    sub.dispatch_tick();

    let lines = logged(&sub);
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
    sub.dispatch_key("C-x");
    assert_eq!(logged(&sub), vec!["key nil"]);
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
    sub.dispatch_key("C-x");
    let ops: Vec<Op> = sub.drain_ops().into_iter().map(|(_, op)| op).collect();
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
    sub.dispatch_key("C-x");
    let lines = logged(&sub);
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

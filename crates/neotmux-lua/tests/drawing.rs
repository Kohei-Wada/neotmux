use std::fs;

use neotmux_core::proto::{DrawCmd, Epoch, PaneId, Rect, Size};
use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;

fn layer(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    dir
}

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = layer(files);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn frame(sub: &LuaSubsystem, rows: u16) {
    let size = Size { rows, cols: 80 };
    let pane = PaneId::from(1);
    let rect = Rect::new(0, 0, 80, rows);
    sub.begin_frame(
        Epoch::new(size, vec![pane], Some(pane)),
        size,
        vec![(pane, rect)],
        Some(pane),
    );
    sub.dispatch_tick();
}

#[test]
fn a_tick_records_what_it_draws() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 1, 2, 'hi') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        DrawCmd::Text { x, y, text, .. } => {
            assert_eq!((*x, *y), (1, 2));
            assert_eq!(text, "hi");
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn ctx_size_reports_the_surface() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.sizer')"),
        (
            "lua/plugin/sizer.lua",
            "neotmux.on_tick(function(ctx) local w, h = ctx:size() ctx:text(nil, 0, 0, w .. 'x' .. h) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    match &cmds[0] {
        DrawCmd::Text { text, .. } => assert_eq!(text, "80x24"),
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn pane_rect_answers_for_a_live_pane_and_nil_otherwise() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.rects')"),
        (
            "lua/plugin/rects.lua",
            r#"
            neotmux.on_tick(function(ctx)
              local r = ctx:pane_rect(1)
              ctx:text(nil, 0, 0, r and (r.w .. 'x' .. r.h) or 'nil')
              ctx:text(nil, 0, 1, ctx:pane_rect(99) and 'some' or 'nil')
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    let texts: Vec<String> = cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["80x24".to_string(), "nil".to_string()]);
}

#[test]
fn a_status_fragment_reaches_the_host() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.clock')"),
        (
            "lua/plugin/clock.lua",
            "neotmux.on_tick(function(ctx) ctx:status('12:34') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let (_, status) = sub.frame_output();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].text, "12:34");
}

#[test]
fn a_tick_that_raises_keeps_its_previous_frame() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.flaky')"),
        (
            "lua/plugin/flaky.lua",
            r#"
            local n = 0
            neotmux.on_tick(function(ctx)
              n = n + 1
              ctx:text(nil, 0, 0, 'frame' .. n)
              if n == 2 then error('boom') end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    let (first, _) = sub.frame_output();
    frame(&sub, 24);
    let (second, _) = sub.frame_output();
    assert_eq!(first, second, "the failed tick replaced the good frame");
}

#[test]
fn a_tick_error_is_reported_with_the_source_name() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.flaky')"),
        (
            "lua/plugin/flaky.lua",
            "neotmux.on_tick(function() error('boom') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 1, "expected one op, got {ops:?}");
    assert_eq!(
        sub.sources().name(ops[0].0).as_deref(),
        Some("plugin/flaky")
    );
}

#[test]
fn a_resize_makes_the_previous_frame_inapplicable() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            r#"
            local drawn = false
            neotmux.on_tick(function(ctx)
              if not drawn then ctx:text(nil, 0, 0, 'once') drawn = true end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(sub.frame_output().0.len(), 1);
    frame(&sub, 25);
    assert!(
        sub.frame_output().0.is_empty(),
        "a stale frame was replayed after a resize"
    );
}

#[test]
fn a_stashed_ctx_cannot_draw_outside_its_tick() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.thief')"),
        (
            "lua/plugin/thief.lua",
            r#"
            local stolen
            neotmux.on_tick(function(ctx)
              if stolen then
                local ok = pcall(function() stolen:text(nil, 0, 0, 'late') end)
                ctx:text(nil, 0, 0, ok and 'allowed' or 'refused')
              else
                stolen = ctx
              end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    frame(&sub, 24);
    let (cmds, _) = sub.frame_output();
    match &cmds[0] {
        DrawCmd::Text { text, .. } => assert_eq!(text, "refused"),
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn same_frame_cross_source_isolation_survives_dispatch_tick() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "require('plugin.healthy') require('plugin.raiser')",
        ),
        (
            "lua/plugin/healthy.lua",
            r#"
            local n = 0
            neotmux.on_tick(function(ctx)
              n = n + 1
              ctx:text(nil, 0, 0, 'healthy' .. n)
            end)
            return {}
            "#,
        ),
        (
            "lua/plugin/raiser.lua",
            r#"
            local n = 0
            neotmux.on_tick(function(ctx)
              n = n + 1
              ctx:text(nil, 0, 1, 'raiser' .. n)
              if n == 2 then error('boom') end
            end)
            return {}
            "#,
        ),
    ]);

    frame(&sub, 24);
    frame(&sub, 24);

    let (cmds, _) = sub.frame_output();
    let texts: Vec<(u16, u16, String)> = cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { x, y, text, .. } => Some((*x, *y, text.clone())),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&(0, 0, "healthy2".to_string())),
        "the healthy plugin's current frame is missing: {texts:?}"
    );
    assert!(
        texts.contains(&(0, 1, "raiser1".to_string())),
        "the raising plugin should still show its last good frame: {texts:?}"
    );
    assert!(
        !texts.iter().any(|(_, _, t)| t == "raiser2"),
        "the raising tick's half-drawn frame leaked through: {texts:?}"
    );

    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 1, "expected exactly one error op, got {ops:?}");
    assert_eq!(
        sub.sources().name(ops[0].0).as_deref(),
        Some("plugin/raiser")
    );
}

#[test]
fn a_bad_colour_raises_at_the_call() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'x', { fg = 'not a colour' }) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 1, "a bad colour should have raised");
}

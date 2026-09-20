use std::fs;

use neotmux_lua::runtime::Layers;
use neotmux_lua::{Geometry, KeyOutcome, LuaSubsystem};
use neotmux_proto::{Color, DrawCmd, Epoch, PaneId, Rect, Size};

mod support;

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
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn frame(sub: &LuaSubsystem, rows: u16) {
    let size = Size { rows, cols: 80 };
    let pane = PaneId::from(1);
    let rect = Rect::new(0, 0, 80, rows);
    sub.begin_frame(
        Epoch::new(size, vec![pane], Some(pane)),
        Geometry {
            size,
            panes: vec![(pane, rect)],
            focus: Some(pane),
            ..Default::default()
        },
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
    let cmds = sub.frame_output();
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
            "neotmux.on_tick(function(ctx) local s = ctx:size() ctx:text(nil, 0, 0, s.cols .. 'x' .. s.rows) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let cmds = sub.frame_output();
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
    let cmds = sub.frame_output();
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
    let first = sub.frame_output();
    frame(&sub, 24);
    let second = sub.frame_output();
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
    assert_eq!(sub.frame_output().len(), 1);
    frame(&sub, 25);
    assert!(
        sub.frame_output().is_empty(),
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
    let cmds = sub.frame_output();
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

    let cmds = sub.frame_output();
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

#[test]
fn a_bg_on_line_is_refused_rather_than_discarded() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.liner')"),
        (
            "lua/plugin/liner.lua",
            "neotmux.on_tick(function(ctx) ctx:line(nil, 0, 0, 3, 0, { bg = 1 }) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 1, "a bg on line should have raised");
    assert!(sub.frame_output().is_empty(), "the line was still drawn");
}

#[test]
fn a_line_still_takes_an_fg() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.liner')"),
        (
            "lua/plugin/liner.lua",
            "neotmux.on_tick(function(ctx) ctx:line(nil, 0, 0, 3, 0, { fg = 4 }) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    assert!(sub.drain_ops().is_empty());
    match &sub.frame_output()[..] {
        [DrawCmd::Line { fg, .. }] => assert_eq!(*fg, Color::Idx(4)),
        other => panic!("unexpected commands: {other:?}"),
    }
}

#[test]
fn a_line_drawn_with_no_glyph_keeps_the_one_it_always_had() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.liner')"),
        (
            "lua/plugin/liner.lua",
            "neotmux.on_tick(function(ctx) ctx:line(nil, 0, 0, 3, 0) end) return {}",
        ),
    ]);
    frame(&sub, 24);
    assert!(sub.drain_ops().is_empty());
    match &sub.frame_output()[..] {
        [DrawCmd::Line { glyph, .. }] => assert_eq!(*glyph, '#'),
        other => panic!("unexpected commands: {other:?}"),
    }
}

/// The glyph is the caller's, and a box-drawing character is three bytes of one: the host
/// must carry the character, not the first byte of it.
#[test]
fn a_line_draws_with_the_glyph_it_was_given() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.liner')"),
        (
            "lua/plugin/liner.lua",
            "neotmux.on_tick(function(ctx) ctx:line(nil, 0, 0, 3, 0, nil, '\u{2502}') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    assert!(sub.drain_ops().is_empty());
    match &sub.frame_output()[..] {
        [DrawCmd::Line { glyph, .. }] => assert_eq!(*glyph, '\u{2502}'),
        other => panic!("unexpected commands: {other:?}"),
    }
}

#[test]
fn a_glyph_of_more_than_one_character_is_refused_rather_than_truncated() {
    for glyph in ["'->'", "''"] {
        let (_dir, sub) = built(&[
            ("init.lua", "require('plugin.liner')"),
            (
                "lua/plugin/liner.lua",
                &format!(
                    "neotmux.on_tick(function(ctx) ctx:line(nil, 0, 0, 3, 0, nil, {glyph}) end) return {{}}"
                ),
            ),
        ]);
        frame(&sub, 24);
        let ops = sub.drain_ops();
        assert_eq!(ops.len(), 1, "{glyph} should have raised");
        assert!(
            sub.frame_output().is_empty(),
            "{glyph} was still drawn with"
        );
    }
}

fn tinter(body: &str) -> (tempfile::TempDir, LuaSubsystem) {
    built(&[
        ("init.lua", "require('plugin.tinter')"),
        (
            "lua/plugin/tinter.lua",
            &format!("neotmux.on_tick(function(ctx) {body} end) return {{}}"),
        ),
    ])
}

#[test]
fn a_tint_records_the_rectangle_and_both_colours() {
    let (_dir, sub) = tinter("ctx:tint(nil, 1, 2, 3, 4, { fg = 7, bg = '#102030' })");
    frame(&sub, 24);
    let cmds = sub.frame_output();
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        DrawCmd::Tint { rect, fg, bg, .. } => {
            assert_eq!(*rect, Rect::new(1, 2, 3, 4));
            assert_eq!(*fg, Some(Color::Idx(7)));
            assert_eq!(*bg, Some(Color::Rgb(0x10, 0x20, 0x30)));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

/// The selection case: change the background, keep whatever coloured the text.
#[test]
fn a_colour_the_tint_does_not_name_is_left_out_of_the_command() {
    let (_dir, sub) = tinter("ctx:tint(nil, 0, 0, 1, 1, { bg = 4 })");
    frame(&sub, 24);
    match &sub.frame_output()[0] {
        DrawCmd::Tint { fg, bg, .. } => {
            assert_eq!(*fg, None);
            assert_eq!(*bg, Some(Color::Idx(4)));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

/// `rect` with no colours erases; a `tint` with none would do nothing, so it says so.
#[test]
fn a_tint_naming_no_colour_is_refused() {
    for colours in ["", ", {}", ", { fg = nil }"] {
        let (_dir, sub) = tinter(&format!("ctx:tint(nil, 0, 0, 1, 1{colours})"));
        frame(&sub, 24);
        let ops = sub.drain_ops();
        assert_eq!(ops.len(), 1, "{colours:?} should have raised");
        assert!(sub.frame_output().is_empty(), "{colours:?} was still drawn");
    }
}

#[test]
fn a_tint_can_name_a_pane_the_way_a_rect_can() {
    let (_dir, sub) = tinter("ctx:tint(1, 0, 0, 2, 2, { bg = 1 })");
    frame(&sub, 24);
    match &sub.frame_output()[0] {
        DrawCmd::Tint { pane, .. } => assert_eq!(*pane, Some(PaneId::from(1))),
        other => panic!("unexpected command: {other:?}"),
    }
}

/// Helper for the per-source commit tests: every `Text` command as (y, text).
fn lines(sub: &LuaSubsystem) -> Vec<(u16, String)> {
    sub.frame_output()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { y, text, .. } => Some((*y, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn two_tick_callbacks_from_one_source_both_reach_the_frame() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.pair')"),
        (
            "lua/plugin/pair.lua",
            r#"
            neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'first') end)
            neotmux.on_tick(function(ctx) ctx:text(nil, 0, 1, 'second') end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![(0, "first".to_string()), (1, "second".to_string())],
        "the first callback's drawing was lost when the second one committed"
    );
}

#[test]
fn one_raising_callback_loses_its_whole_sources_frame_and_keeps_the_previous_one() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "require('plugin.pair') require('plugin.bystander')",
        ),
        (
            "lua/plugin/pair.lua",
            r#"
            local n = 0
            neotmux.on_tick(function(ctx)
              n = n + 1
              ctx:text(nil, 0, 0, 'first' .. n)
            end)
            neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 1, 'second' .. n)
              if n == 2 then error('boom') end
            end)
            return {}
            "#,
        ),
        (
            "lua/plugin/bystander.lua",
            r#"
            local n = 0
            neotmux.on_tick(function(ctx)
              n = n + 1
              ctx:text(nil, 0, 2, 'bystander' .. n)
            end)
            return {}
            "#,
        ),
    ]);

    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![
            (0, "first1".to_string()),
            (1, "second1".to_string()),
            (2, "bystander1".to_string()),
        ]
    );

    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![
            (0, "first1".to_string()),
            (1, "second1".to_string()),
            (2, "bystander2".to_string()),
        ],
        "a raising callback must lose its source's whole frame, and only its own source's"
    );
}

#[test]
fn grouping_tick_dispatch_by_source_still_composes_sources_in_load_order() {
    // `later`'s tick has the lower RegistrationId and `earlier` the lower SourceId, so a frame
    // composed in registration order would reverse them; composition is by load order.
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "require('plugin.earlier') require('plugin.later')",
        ),
        (
            "lua/plugin/earlier.lua",
            r#"
            local m = require('plugin.later')
            neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'earlier') end)
            return {}
            "#,
        ),
        (
            "lua/plugin/later.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 1, 'later') end) return {}",
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![(0, "earlier".to_string()), (1, "later".to_string())]
    );
}

#[test]
fn removing_the_last_tick_registration_stops_the_last_frame_replaying() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.quitter')"),
        (
            "lua/plugin/quitter.lua",
            r#"
            local h = neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'transient') end)
            neotmux.keymap.set('C-q', function() h:remove() return true end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(lines(&sub), vec![(0, "transient".to_string())]);

    assert_eq!(sub.dispatch_key("C-q"), KeyOutcome::Consumed);
    frame(&sub, 24);
    assert!(
        lines(&sub).is_empty(),
        "a removed registration's last frame is still being replayed: {:?}",
        lines(&sub)
    );
}

/// A source that removes its own last tick registration from inside that tick loses the
/// frame it drew in the same tick: `forget` clears its scratch along with its committed
/// entry, and the per-source commit that follows has nothing left to commit. Decided
/// rather than incidental (§4.5) — a source with no tick left cannot refresh a frame, so
/// showing the one it drew on its way out would be showing something it can never update.
#[test]
fn removing_the_last_tick_registration_from_inside_it_loses_that_frame_too() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.leaver')"),
        (
            "lua/plugin/leaver.lua",
            r#"
            local h
            h = neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, 'last words')
              h:remove()
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert!(
        lines(&sub).is_empty(),
        "the frame drawn on the way out was kept: {:?}",
        lines(&sub)
    );
    frame(&sub, 24);
    assert!(lines(&sub).is_empty(), "and it must not come back");
}

#[test]
fn removing_one_of_two_tick_registrations_keeps_the_source_drawing() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.pair')"),
        (
            "lua/plugin/pair.lua",
            r#"
            local h
            h = neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, 'going')
              h:remove()
            end)
            neotmux.on_tick(function(ctx) ctx:text(nil, 0, 1, 'staying') end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![(1, "staying".to_string())],
        "forgetting fired although the source still has a tick registration"
    );
}

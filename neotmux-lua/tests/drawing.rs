use std::fs;

use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{Color, DrawCmd, Epoch, Geometry, PaneId, Rect, Size};

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
        support::one(),
        Epoch::new(size, vec![pane], Some(pane)),
        Geometry {
            area: Rect::new(0, 0, size.cols, size.rows),
            panes: vec![(pane, rect)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());
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
    let ops = sub.drain_ops(Some(support::one()));
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

    let ops = sub.drain_ops(Some(support::one()));
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
    let ops = sub.drain_ops(Some(support::one()));
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
    let ops = sub.drain_ops(Some(support::one()));
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
    assert!(sub.drain_ops(Some(support::one())).is_empty());
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
    assert!(sub.drain_ops(Some(support::one())).is_empty());
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
    assert!(sub.drain_ops(Some(support::one())).is_empty());
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
        let ops = sub.drain_ops(Some(support::one()));
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
        let ops = sub.drain_ops(Some(support::one()));
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

    assert_eq!(
        sub.dispatch_key(support::one(), "C-q"),
        KeyOutcome::Consumed
    );
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

// `neotmux.float` and its handle (Task 3, spec §2, §3, §7). Nothing draws into a float
// yet (Task 4): these tests observe the grant and the handle's answers the way
// `reserve.rs`'s tests do, by having the plugin write what it saw into its own frame
// with `ctx:text`, read back through `lines`.

#[test]
fn a_grant_from_a_tick_answers_a_handle_and_the_granted_rect_is_recorded() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            neotmux.on_tick(function(ctx)
              local win, message = neotmux.float({ x = 2, y = 3, w = 8, h = 4 })
              ctx:text(nil, 0, 0, tostring(win ~= nil) .. ':' .. tostring(message))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(lines(&sub), vec![(0, "true:nil".to_string())]);

    // The plugin's own answer only proves the call did not raise; the rect itself is
    // echoed back from the registry the grant wrote into, keyed by the plugin's source id.
    let owner = sub
        .sources()
        .interned()
        .into_iter()
        .find(|(_, name)| name == "plugin/floater")
        .map(|(id, _)| id)
        .expect("the plugin should have been interned as a source");
    let granted: Vec<Rect> = sub.floats().by_owner(owner).into_values().collect();
    assert_eq!(
        granted,
        vec![Rect::new(2, 3, 8, 4)],
        "the grant should have recorded exactly the rect it was given"
    );
}

#[test]
fn a_bad_grant_answers_nil_and_a_message_and_does_not_stop_a_good_grant() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            neotmux.on_tick(function(ctx)
              local row = 0
              local function check(opts)
                local win, message = neotmux.float(opts)
                ctx:text(nil, 0, row, tostring(win == nil) .. ':' .. tostring(message ~= nil))
                row = row + 1
              end
              check({ y = 0, w = 1, h = 1 })         -- missing x
              check({ x = 0, y = 0, w = 1, h = 0 })  -- zero h
              check({ x = -1, y = 0, w = 1, h = 1 }) -- negative x
              check({ x = 0, y = 0, w = 1.5, h = 1 }) -- non-integer w
              local win, message = neotmux.float({ x = 0, y = 0, w = 1, h = 1 })
              ctx:text(nil, 0, row, tostring(win ~= nil) .. ':' .. tostring(message))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![
            (0, "true:true".to_string()),
            (1, "true:true".to_string()),
            (2, "true:true".to_string()),
            (3, "true:true".to_string()),
            (4, "true:nil".to_string()),
        ],
        "every bad grant should answer nil plus a message, and the well-formed grant \
         after them should still succeed"
    );
}

#[test]
fn a_grant_whose_extent_reaches_past_the_surface_coordinates_answers_nil() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            neotmux.on_tick(function(ctx)
              local row = 0
              local function check(x, y, w, h, label)
                local win, message = neotmux.float({ x = x, y = y, w = w, h = h })
                ctx:text(nil, 0, row, label .. ':' .. tostring(win == nil) .. ':' .. tostring(message ~= nil))
                row = row + 1
              end
              check(65534, 0, 65535, 1, 'x+w')   -- 65534 + 65535 reaches past 65535
              check(0, 65534, 1, 65535, 'y+h')   -- the same, on the other axis
              local win, message = neotmux.float({ x = 98, y = 20, w = 8, h = 4 })
              ctx:text(nil, 0, row, 'past-the-surface:' .. tostring(win ~= nil) .. ':' .. tostring(message ~= nil))
              row = row + 1
              local win2 = neotmux.float({ x = 0, y = 0, w = 4, h = 2 })
              ctx:text(nil, 0, row, 'good-after:' .. tostring(win2 ~= nil))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![
            (0, "x+w:true:true".to_string()),
            (1, "y+h:true:true".to_string()),
            (2, "past-the-surface:true:false".to_string()),
            (3, "good-after:true".to_string()),
        ],
        "a rect whose right or bottom edge reaches past the u16 surface coordinate space \
         (65534 + 65535) must answer nil plus a message whose substance is 'reach past the \
         surface's coordinates' — refused, not an overflow panic; `false` on the \
         past-the-surface row is the nil second answer of a successful grant (a success \
         pair is handle-plus-nil, so the `message ~= nil` flag reads false); a rect that \
         merely reaches past the surface's present edge stays granted, and the good grant \
         after the bad ones still works: {:?}",
        lines(&sub)
    );
}

#[test]
fn the_handles_move_to_and_resize_to_answer_true_or_nil_and_a_message() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            neotmux.on_tick(function(ctx)
              local win = neotmux.float({ x = 0, y = 0, w = 4, h = 4 })
              local ok, err = win:move_to(3, 2)
              ctx:text(nil, 0, 0, tostring(ok) .. ':' .. tostring(err))
              local ok2, err2 = win:move_to(-1, 0)
              ctx:text(nil, 0, 1, tostring(ok2 == nil) .. ':' .. tostring(err2 ~= nil))
              local ok3, err3 = win:resize_to(5, 6)
              ctx:text(nil, 0, 2, tostring(ok3) .. ':' .. tostring(err3))
              local ok4, err4 = win:resize_to(0, 1)
              ctx:text(nil, 0, 3, tostring(ok4 == nil) .. ':' .. tostring(err4 ~= nil))
              win:remove()
              win:remove()
              ctx:text(nil, 0, 4, 'removed twice without raising')
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![
            (0, "true:nil".to_string()),
            (1, "true:true".to_string()),
            (2, "true:nil".to_string()),
            (3, "true:true".to_string()),
            (4, "removed twice without raising".to_string()),
        ]
    );
}

/// The extent bound applies to the handle too, not just the grant: a move or resize that
/// is legal field by field can still reach past the u16 surface coordinate space once the
/// float's other half is added (65534 + a width of 4, 2 + a height of 65535) — those answer
/// `nil` plus a message (substance: "together reach past the surface's coordinates") and
/// move nothing; a legal move in between and afterwards still lands, read the rect back
/// with `ctx:float_rect`.
#[test]
fn a_move_or_resize_reaching_past_the_surface_coordinates_answers_nil_and_moves_nothing() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win, message = neotmux.float({ x = 0, y = 0, w = 4, h = 4 })
            if win == nil then error('neotmux.float failed: ' .. tostring(message)) end
            neotmux.on_tick(function(ctx)
              local row = 0
              local function put(ok, err) ctx:text(nil, 0, row, tostring(ok) .. ':' .. tostring(err ~= nil)) row = row + 1 end
              local m1, e1 = win:move_to(65534, 0)      -- 65534 + 4 reaches past 65535
              put(m1 == nil, e1)
              local m2, e2 = win:move_to(0, 65535)      -- 65535 + 4, the other axis
              put(m2 == nil, e2)
              local m3, e3 = win:move_to(2, 2)          -- legal: 2 + 4 fits on both axes
              put(m3 ~= nil, e3)
              local m4, e4 = win:resize_to(65535, 4)    -- from x = 2: 2 + 65535 reaches past
              put(m4 == nil, e4)
              local m5, e5 = win:resize_to(4, 65535)    -- from y = 2: 2 + 65535 reaches past
              put(m5 == nil, e5)
            end)
            neotmux.on_tick(function(ctx)
              local r = ctx:float_rect(win)
              if r then ctx:text(nil, 0, 0, r.x .. ',' .. r.y .. ',' .. r.w .. ',' .. r.h) end
            end)
            return {}
            "#,
        ),
    ]);
    let expected = vec![
        (0, "true:true".to_string()),
        (1, "true:true".to_string()),
        (2, "true:false".to_string()),
        (3, "true:true".to_string()),
        (4, "true:true".to_string()),
        (0, "2,2,4,4".to_string()),
    ];
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        expected,
        "a refused move or resize must answer nil plus a message whose substance is '\\n\
         together reach past the surface's coordinates' and change nothing, the legal move\\n\
         answers true, and the rect read back is the legal move's (2,2) at the original\\n\
         size: {:?}",
        lines(&sub)
    );
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        expected,
        "and it must stay refused, not start landing on later frames: {:?}",
        lines(&sub)
    );
}

/// The spec's past-the-surface clause stays true through mutation, not just grant: a
/// `move_to` or `resize_to` that is off the 80x24 surface but inside the u16 extent still
/// answers `true` and lands.
#[test]
fn a_legal_move_and_resize_past_the_surface_still_land() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win, message = neotmux.float({ x = 2, y = 3, w = 8, h = 4 })
            if win == nil then error('neotmux.float failed: ' .. tostring(message)) end
            neotmux.on_tick(function(ctx)
              local m1, e1 = win:move_to(200, 80)        -- past the surface, inside the space
              ctx:text(nil, 0, 0, 'move:' .. tostring(m1 ~= nil) .. ':' .. tostring(e1 ~= nil))
              local m2, e2 = win:resize_to(4000, 4000)  -- from (200, 80): both pairs fit
              ctx:text(nil, 0, 1, 'resize:' .. tostring(m2 ~= nil) .. ':' .. tostring(e2 ~= nil))
            end)
            neotmux.on_tick(function(ctx)
              local r = ctx:float_rect(win)
              if r then ctx:text(nil, 0, 2, r.x .. ',' .. r.y .. ',' .. r.w .. ',' .. r.h) end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![
            (0, "move:true:false".to_string()),
            (1, "resize:true:false".to_string()),
            (2, "200,80,4000,4000".to_string()),
        ],
        "a move or resize that is merely off the surface's edge stays legal: the answers \
         are the true-plus-nil pairs, and both mutations land where they were asked: \
         {:?}",
        lines(&sub)
    );
}

/// Modelled on `subsystem.rs`'s `a_failed_loads_rollback_does_not_reach_a_callback_it_got
/// _from_a_library`: the failing chunk's own float grant is rolled back by the require
/// wrapper's `floats.remove_all_of` (mirroring what it already does for registrations and
/// reservations), so a plugin that survives the load is unaffected — its own grant answers
/// exactly as it would have with no failed load before it. Nothing draws into a float until
/// Task 4, so what is observable here is the wiring, not a drawn leak; Task 4's ctx tests
/// cover the draw-side of "a float the owner no longer holds is dropped".
#[test]
fn a_failed_loads_own_float_grant_does_not_survive_the_rollback() {
    let dir = layer(&[
        ("init.lua", "require('plugin.half')"),
        (
            "lua/plugin/lib.lua",
            "local M = {} function M.tick(ctx) local win, message = \
             neotmux.float({ x = 0, y = 0, w = 1, h = 1 }) ctx:text(nil, 0, 0, \
             tostring(win ~= nil) .. ':' .. tostring(message)) end return M",
        ),
        (
            "lua/plugin/half.lua",
            r#"
            local lib = require('plugin.lib')
            neotmux.float({ x = 0, y = 0, w = 2, h = 2 })
            neotmux.on_tick(lib.tick)
            error('boom')
            "#,
        ),
    ]);
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1, "{failures:?}");

    frame(&sub, 24);
    assert_eq!(
        lines(&sub),
        vec![(0, "true:nil".to_string())],
        "the surviving plugin's own grant should succeed exactly as it would with no prior \
         failed grant"
    );
    // The observable proof the reap happened: `half.lua`'s own grant (made at load time,
    // before it errored) is gone, and the only live float is `lib.lua`'s, granted this
    // frame by the tick that survived the rollback. Without `floats.remove_all_of(id)` in
    // the require wrapper's failure branch this would be 2.
    assert_eq!(
        sub.floats().count(),
        1,
        "the failed load's own float grant should not have survived the rollback"
    );
}

// Task 4: drawing into a float, and the layering it produces (spec §4). `ctx.floats` is
// captured once per callback, before that callback runs (matching `band`): so a float
// granted inside a callback is invisible to that same callback's own draws — it lands on
// the next frame — while a `remove` inside a callback only reaches the callbacks that run
// after it, the one it runs in keeps drawing into the float.

/// One tick's worth of `neotmux.log.info` lines, in emission order.
fn log_lines(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_draw_into_a_float_is_offset_and_clipped_per_verb() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win
            neotmux.on_tick(function(ctx)
              if win == nil then
                win = neotmux.float({ x = 3, y = 2, w = 8, h = 4 })
                return
              end
              ctx:text(win, 1, 1, 'abcdefghij')
              ctx:rect(win, 1, 1, 10, 10, { fg = 1, bg = 2 })
              ctx:tint(win, 1, 1, 10, 10, { fg = 3 })
              ctx:line(win, 1, 1, 3, 1)
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24); // grants the float; nothing drawn yet, ctx.floats did not see it
    frame(&sub, 24); // ctx.floats now carries it, so this tick's four draws all land
    assert_eq!(
        sub.frame_output(),
        vec![
            DrawCmd::Text {
                pane: None,
                x: 4,
                y: 3,
                text: "abcdefg".to_string(),
                fg: Color::Default,
                bg: Color::Default,
            },
            DrawCmd::Rect {
                pane: None,
                rect: Rect::new(4, 3, 7, 3),
                fg: Color::Idx(1),
                bg: Color::Idx(2),
            },
            DrawCmd::Tint {
                pane: None,
                rect: Rect::new(4, 3, 7, 3),
                fg: Some(Color::Idx(3)),
                bg: None,
            },
            DrawCmd::Line {
                pane: None,
                from: (4, 3),
                to: (6, 3),
                glyph: '#',
                fg: Color::Default,
            },
        ],
        "each command lands offset by the float's origin (3, 2); the text, which would \
         read 10 columns wide at local x=1 against an 8-wide float, is truncated to the \
         cap of 7, not silently dropped or left full-length"
    );
}

#[test]
fn a_float_off_the_surface_edge_still_arrives_whole() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win = neotmux.float({ x = 98, y = 20, w = 8, h = 4 })
            neotmux.on_tick(function(ctx) ctx:text(win, 0, 0, 'hi') end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        sub.frame_output(),
        vec![DrawCmd::Text {
            pane: None,
            x: 98,
            y: 20,
            text: "hi".to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }],
        "only the float-local clip runs host-side; the command arrives whole even though \
         (98, 20) falls outside the 80x24 surface -- clipping to the surface is the \
         renderer's job, not this layer's"
    );
}

#[test]
fn a_shrink_from_inside_a_tick_clips_the_next_frame_draw() {
    // `resize_to` inside a tick lands on the next frame (the ctx snapshot the draws take on is
    // this tick's rect), so the tick's own draw still goes out at the old width, and the next
    // one is clipped to the new one — the band's `resize_to` bargain, held for a float too.
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win, message = neotmux.float({ x = 1, y = 1, w = 8, h = 4 })
            if win == nil then error('neotmux.float failed: ' .. tostring(message)) end
            neotmux.on_tick(function(ctx)
              ctx:text(win, 0, 0, 'abcdefghijkl')
              local ok, err = win:resize_to(4, 4)
              assert(ok, 'resize_to failed: ' .. tostring(err))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    let wide: Vec<String> = sub
        .frame_output()
        .into_iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(
        wide,
        vec!["abcdefgh".to_string()],
        "the tick's own draw must be clipped to the 8-wide rect it started in: a shrink \
         issued in the same tick is next frame's problem, not this draw's: {wide:?}"
    );
    frame(&sub, 24);
    let narrow: Vec<String> = sub
        .frame_output()
        .into_iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(
        narrow,
        vec!["abcd".to_string()],
        "the next frame's draw of the same 12 characters is clipped to the new 4-wide rect: \
         {narrow:?}"
    );
}

#[test]
fn grant_order_not_draw_order_decides_the_float_z_order() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        (
            "lua/plugin/a.lua",
            r#"
            local win
            neotmux.on_tick(function(ctx)
              if win == nil then
                win = neotmux.float({ x = 0, y = 0, w = 4, h = 1 })
                return
              end
              ctx:text(win, 0, 0, 'A')
            end)
            return {}
            "#,
        ),
        (
            "lua/plugin/b.lua",
            r#"
            local win = neotmux.float({ x = 0, y = 0, w = 4, h = 1 })
            neotmux.on_tick(function(ctx) ctx:text(win, 0, 0, 'B') end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    frame(&sub, 24);
    let texts: Vec<String> = sub
        .frame_output()
        .into_iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec!["B".to_string(), "A".to_string()],
        "b.lua grants its float while loading, before any tick runs, so it gets the \
         smaller float id even though a.lua loaded first and its tick is dispatched \
         first every frame; z-order follows grant (float id) order, not dispatch order"
    );
}

#[test]
fn a_draw_into_a_float_the_source_no_longer_owns_is_dropped() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win = neotmux.float({ x = 0, y = 0, w = 10, h = 4 })
            neotmux.on_tick(function(ctx)
              ctx:text(win, 0, 0, 'first')
              win:remove()
            end)
            neotmux.on_tick(function(ctx)
              ctx:text(win, 0, 1, 'second')
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    let texts: Vec<String> = sub
        .frame_output()
        .into_iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec!["first".to_string()],
        "the first callback's own draw, made while it still owned the float, arrives; \
         the second callback's ctx captured the float list after the first callback \
         removed it, so its draw into the same handle is silently dropped"
    );
}

/// The capture is per callback, not per frame, and it is taken *before* the callback runs:
/// a `remove` inside a callback cannot reach a draw the same callback makes — the one it
/// runs in keeps holding the float it was handed — and it reaches every callback that comes
/// after it. `a_draw_into_a_float_the_source_no_longer_owns_is_dropped` pins the second
/// half across two callbacks; this one pins the first half inside one callback.
#[test]
fn a_remove_inside_a_callback_keeps_the_callback_own_draws_in_that_one_frame_only() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win = neotmux.float({ x = 0, y = 0, w = 10, h = 4 })
            neotmux.on_tick(function(ctx)
              ctx:text(win, 0, 0, 'before')
              win:remove()
              ctx:text(win, 2, 1, 's')
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    let placed: Vec<(u16, u16, String)> = sub
        .frame_output()
        .into_iter()
        .filter_map(|c| match c {
            DrawCmd::Text { x, y, text, .. } => Some((x, y, text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        placed,
        vec![(0, 0, "before".to_string()), (2, 1, "s".to_string())],
        "the callback's ctx was captured before it ran, so its draw after the remove still \
         lands this frame, at the float's own coordinates: {placed:?}"
    );
    frame(&sub, 24);
    assert!(
        sub.frame_output().is_empty(),
        "the remove reaches the next frame's capture of the same callback, so the frame \
         it draws in is dropped: {:?}",
        sub.frame_output()
    );
}

#[test]
fn ctx_float_rect_reports_the_live_rect_and_nil_once_removed() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            local win = neotmux.float({ x = 3, y = 2, w = 8, h = 4 })
            local removed = false
            neotmux.on_tick(function(ctx)
              local r = ctx:float_rect(win)
              if r then
                neotmux.log.info(r.x .. ',' .. r.y .. ',' .. r.w .. ',' .. r.h)
              else
                neotmux.log.info('nil')
              end
              if not removed then
                win:remove()
                removed = true
              end
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(log_lines(&sub), vec!["3,2,8,4".to_string()]);
    frame(&sub, 24);
    assert_eq!(log_lines(&sub), vec!["nil".to_string()]);
}

#[test]
fn ctx_float_rect_refuses_a_pane_or_nil_target() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.floater')"),
        (
            "lua/plugin/floater.lua",
            r#"
            neotmux.on_tick(function(ctx)
              local ok = pcall(function() return ctx:float_rect(nil) end)
              neotmux.log.info('nil:' .. tostring(ok))
              local ok2 = pcall(function() return ctx:float_rect(1) end)
              neotmux.log.info('pane:' .. tostring(ok2))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, 24);
    assert_eq!(
        log_lines(&sub),
        vec!["nil:false".to_string(), "pane:false".to_string()],
        "float_rect answers only for a float handle; neither nil nor a pane number should \
         be accepted"
    );
}

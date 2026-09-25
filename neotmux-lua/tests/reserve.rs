//! `neotmux.reserve` and `ctx:reserved()` across the ABI: what a source is granted, what it
//! is refused, and which band it draws in.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Reserved, Size};

mod support;

const SURFACE: Size = Size { rows: 24, cols: 80 };

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

/// What the host does at a frame boundary: settle the surface, hand out the bands, publish
/// the geometry the panes were left with, and tick.
fn frame(sub: &LuaSubsystem, applied: bool) {
    let reserved = if applied {
        sub.reserved()
    } else {
        Reserved::default()
    };
    sub.lay_out_reservations(support::one(), SURFACE, reserved);
    let content = Size {
        rows: SURFACE.rows - reserved.rows(),
        cols: SURFACE.cols - reserved.cols(),
    };
    let pane = PaneId::from(1);
    let rect = Rect::new(reserved.left, reserved.top, content.cols, content.rows);
    sub.begin_frame(
        support::one(),
        Epoch::new(content, vec![pane], Some(pane)),
        Geometry {
            area: rect,
            panes: vec![(pane, rect)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());
}

/// Every `Text` command's text, in the order the frame composed them.
fn texts(sub: &LuaSubsystem) -> Vec<String> {
    sub.frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn reporter(name: &str) -> String {
    band_reporter(name, "bottom", 1)
}

/// A plugin that takes a band of `edge` and reports where it landed. `y+h` for a horizontal
/// band, `x+w` for a vertical one — each axis reported in the coordinate it moves along.
fn band_reporter(name: &str, edge: &str, cells: u16) -> String {
    let along = match edge {
        "left" | "right" => ("x", "w"),
        _ => ("y", "h"),
    };
    let (pos, size) = along;
    format!(
        r#"
        neotmux.reserve('{edge}', {cells})
        neotmux.on_tick(function(ctx)
          local band = ctx:reserved()
          ctx:text(nil, 0, 0, '{name}=' .. (band and (band.{pos} .. '+' .. band.{size}) or 'nil'))
        end)
        return {{}}
        "#
    )
}

/// The assertion the whole per-source rule rests on: two sources reserving the same edge get
/// different rows, allocated from the outside in, and each sees only its own.
#[test]
fn two_sources_reserving_one_edge_get_separate_bands() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", &reporter("a")),
        ("lua/plugin/b.lua", &reporter("b")),
    ]);
    assert_eq!(sub.reserved(), Reserved::horizontal(0, 2));
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["a=23+1".to_string(), "b=22+1".to_string()],
        "the two sources were handed the same band"
    );
}

#[test]
fn a_source_that_reserved_nothing_has_no_band() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.idle')"),
        ("lua/plugin/a.lua", &reporter("a")),
        (
            "lua/plugin/idle.lua",
            r#"
            neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, 'idle=' .. tostring(ctx:reserved()))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["a=23+1".to_string(), "idle=nil".to_string()]
    );
}

/// The §6 case one level up: the terminal shrank below what the bands need, so none is
/// applied and the owner is told so rather than drawing over a pane.
#[test]
fn a_surface_that_cannot_afford_the_band_reports_nil() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a')"),
        ("lua/plugin/a.lua", &reporter("a")),
    ]);
    frame(&sub, false);
    assert_eq!(texts(&sub), vec!["a=nil".to_string()]);
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["a=23+1".to_string()],
        "growing back brings the band back"
    );
}

#[test]
fn a_band_reserved_during_a_tick_does_not_move_that_frames_bands() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.late')"),
        ("lua/plugin/a.lua", &reporter("a")),
        (
            "lua/plugin/late.lua",
            r#"
            local asked = false
            neotmux.on_tick(function(ctx)
              if not asked then
                asked = true
                neotmux.reserve('bottom', 1)
              end
              ctx:text(nil, 0, 0, 'late=' .. tostring(ctx:reserved() and 'band' or 'nil'))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["a=23+1".to_string(), "late=nil".to_string()],
        "a reservation made mid-tick was applied to the frame it was made in"
    );
    assert_eq!(
        sub.reserved(),
        Reserved::horizontal(0, 2),
        "the host is told about it for the next frame"
    );

    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["a=23+1".to_string(), "late=band".to_string()],
        "the next frame is where it takes effect"
    );
}

#[test]
fn losing_the_rows_to_whoever_asked_first_is_a_value_not_an_error() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.first') require('plugin.late')"),
        (
            "lua/plugin/first.lua",
            "neotmux.reserve('bottom', 4) return {}",
        ),
        (
            "lua/plugin/late.lua",
            r#"
            local handle, message = neotmux.reserve('bottom', 1)
            neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, tostring(handle) .. ': ' .. message)
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    let reported = texts(&sub);
    assert_eq!(reported.len(), 1);
    assert!(
        reported[0].starts_with("nil: ") && reported[0].contains("will not fit"),
        "{reported:?}"
    );
    assert_eq!(
        sub.reserved(),
        Reserved::horizontal(0, 4),
        "the refused reservation changed the grant"
    );
}

#[test]
fn releasing_a_band_gives_the_rows_back_and_moves_the_survivor_out() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        (
            "lua/plugin/a.lua",
            r#"
            local handle = neotmux.reserve('bottom', 1)
            neotmux.on_key(function()
              handle:remove()
              return true
            end)
            neotmux.on_tick(function() end)
            return {}
            "#,
        ),
        ("lua/plugin/b.lua", &reporter("b")),
    ]);
    frame(&sub, true);
    assert_eq!(texts(&sub), vec!["b=22+1".to_string()]);

    sub.dispatch_key(support::one(), "C-r");
    assert_eq!(sub.reserved(), Reserved::horizontal(0, 1));
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["b=23+1".to_string()],
        "the survivor did not move out to the edge the removed band held"
    );
}

/// A malformed argument raises, like every other one in the ABI.
#[test]
fn an_edge_that_does_not_exist_raises() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.sideways')"),
        (
            "lua/plugin/sideways.lua",
            r#"
            local ok, err = pcall(neotmux.reserve, 'middle', 1)
            neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, tostring(ok) .. ': ' .. tostring(err))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    let reported = texts(&sub);
    assert!(
        reported[0].starts_with("false: ") && reported[0].contains("unknown edge 'middle'"),
        "{reported:?}"
    );
    assert_eq!(sub.reserved(), Reserved::default());
}

/// The vertical edges, across the ABI: a band measured in columns, answered in the same
/// surface coordinates a horizontal one is.
#[test]
fn a_left_band_is_columns_off_the_left_of_the_surface() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.side')"),
        ("lua/plugin/side.lua", &band_reporter("side", "left", 20)),
    ]);
    assert_eq!(sub.reserved(), Reserved::vertical(20, 0));
    frame(&sub, true);
    assert_eq!(texts(&sub), vec!["side=0+20".to_string()]);
}

#[test]
fn a_right_band_sits_against_the_right_of_the_surface() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.side')"),
        ("lua/plugin/side.lua", &band_reporter("side", "right", 20)),
    ]);
    assert_eq!(sub.reserved(), Reserved::vertical(0, 20));
    frame(&sub, true);
    assert_eq!(texts(&sub), vec!["side=60+20".to_string()]);
}

/// The same outside-in rule the horizontal edges follow, in the other unit.
#[test]
fn two_sources_reserving_one_vertical_edge_get_separate_bands() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", &band_reporter("a", "left", 10)),
        ("lua/plugin/b.lua", &band_reporter("b", "left", 6)),
    ]);
    assert_eq!(sub.reserved(), Reserved::vertical(16, 0));
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["a=0+10".to_string(), "b=10+6".to_string()],
        "the second to ask sits inside the first"
    );
}

/// A band on each axis: the vertical one takes what the horizontal one left, so the two
/// never share a cell.
#[test]
fn a_vertical_band_and_a_horizontal_one_do_not_overlap() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.bar') require('plugin.side')"),
        (
            "lua/plugin/bar.lua",
            r#"
            neotmux.reserve('top', 1)
            neotmux.on_tick(function(ctx)
              local b = ctx:reserved()
              ctx:text(nil, 0, 0, 'bar=' .. b.x .. ',' .. b.y .. ' ' .. b.w .. 'x' .. b.h)
            end)
            return {}
            "#,
        ),
        (
            "lua/plugin/side.lua",
            r#"
            neotmux.reserve('left', 20)
            neotmux.on_tick(function(ctx)
              local b = ctx:reserved()
              ctx:text(nil, 0, 0, 'side=' .. b.x .. ',' .. b.y .. ' ' .. b.w .. 'x' .. b.h)
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["bar=0,0 80x1".to_string(), "side=0,1 20x23".to_string()]
    );
}

/// A count past the edge's own bound raises, and the two bounds are different numbers: four
/// columns is a legal sidebar where forty rows would be a mistake.
#[test]
fn the_column_bound_is_not_the_row_bound() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.probe')"),
        (
            "lua/plugin/probe.lua",
            r#"
            local wide_ok = pcall(neotmux.reserve, 'left', 40)
            local tall_ok, tall_err = pcall(neotmux.reserve, 'top', 40)
            neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, tostring(wide_ok) .. '/' .. tostring(tall_ok)
                .. ': ' .. tostring(tall_err))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    let reported = texts(&sub);
    assert!(
        reported[0].starts_with("true/false: ") && reported[0].contains("at most 4 rows"),
        "{reported:?}"
    );
    assert_eq!(sub.reserved(), Reserved::vertical(40, 0));
}

#[test]
fn a_band_no_edge_could_ever_hold_raises() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.huge')"),
        (
            "lua/plugin/huge.lua",
            r#"
            local ok, err = pcall(neotmux.reserve, 'top', 40)
            neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, tostring(ok) .. ': ' .. tostring(err))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    let reported = texts(&sub);
    assert!(
        reported[0].starts_with("false: ") && reported[0].contains("at most"),
        "{reported:?}"
    );
    assert_eq!(sub.reserved(), Reserved::default());
}

/// A band changes width without being given back: the owner sees the new rect and the panes
/// see the cells, both on the frame after the call.
#[test]
fn a_resize_changes_the_band_and_what_the_panes_get_on_the_next_frame() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.side')"),
        (
            "lua/plugin/side.lua",
            r#"
            local handle = neotmux.reserve('left', 20)
            neotmux.on_key(function()
              handle:resize_to(4)
              return true
            end)
            neotmux.on_tick(function(ctx)
              local band = ctx:reserved()
              ctx:text(nil, 0, 0, 'side=' .. band.x .. '+' .. band.w
                .. ' panes=' .. ctx:size().cols)
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    assert_eq!(texts(&sub), vec!["side=0+20 panes=60".to_string()]);

    sub.dispatch_key(support::one(), "C-b");
    assert_eq!(
        sub.reserved(),
        Reserved::vertical(4, 0),
        "the host is told about it for the next frame"
    );
    frame(&sub, true);
    assert_eq!(texts(&sub), vec!["side=0+4 panes=76".to_string()]);
}

/// Why a fold is a resize and not a remove: the band that folds keeps its place on the edge,
/// so the one reserved after it stays inside it both ways.
#[test]
fn a_band_that_folds_and_unfolds_keeps_its_place_on_the_edge() {
    let folder = r#"
        local handle = neotmux.reserve('left', 10)
        local wide = true
        neotmux.on_key(function()
          wide = not wide
          handle:resize_to(wide and 10 or 1)
          return true
        end)
        neotmux.on_tick(function(ctx)
          local band = ctx:reserved()
          ctx:text(nil, 0, 0, 'outer=' .. band.x .. '+' .. band.w)
        end)
        return {}
    "#;
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "require('plugin.outer') require('plugin.inner')",
        ),
        ("lua/plugin/outer.lua", folder),
        ("lua/plugin/inner.lua", &band_reporter("inner", "left", 6)),
    ]);
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["outer=0+10".to_string(), "inner=10+6".to_string()]
    );

    sub.dispatch_key(support::one(), "C-b");
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["outer=0+1".to_string(), "inner=1+6".to_string()],
        "the folded band is still the outer one"
    );

    sub.dispatch_key(support::one(), "C-b");
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["outer=0+10".to_string(), "inner=10+6".to_string()],
        "unfolding puts both back where they were"
    );
}

/// The same distinction `reserve` draws. Growing into cells another plugin holds is a lost
/// race: `nil` plus a message, and the band it already had.
#[test]
fn a_resize_past_the_edges_cap_is_refused_and_changes_nothing() {
    let (_dir, sub) = built(&[
        (
            "init.lua",
            "require('plugin.first') require('plugin.greedy')",
        ),
        (
            "lua/plugin/first.lua",
            "neotmux.reserve('left', 30) return {}",
        ),
        (
            "lua/plugin/greedy.lua",
            r#"
            local handle = neotmux.reserve('left', 10)
            neotmux.on_tick(function(ctx)
              local band = ctx:reserved()
              local ok, message = handle:resize_to(11)
              ctx:text(nil, 0, 0, tostring(ok) .. ': ' .. tostring(message)
                .. ' band=' .. band.x .. '+' .. band.w)
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    let reported = texts(&sub);
    assert!(
        reported[0].starts_with("nil: ") && reported[0].contains("will not fit"),
        "{reported:?}"
    );
    assert!(reported[0].ends_with("band=30+10"), "{reported:?}");
    assert_eq!(
        sub.reserved(),
        Reserved::vertical(40, 0),
        "the refused resize changed the grant"
    );
}

/// A count no edge could hold, and zero, are the typos they are on `reserve`: there is no
/// band of nothing, and a plugin never gets a rect it must not draw in.
#[test]
fn resizing_to_zero_or_past_the_bound_raises() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.side')"),
        (
            "lua/plugin/side.lua",
            r#"
            local handle = neotmux.reserve('left', 10)
            local shut_ok, shut_err = pcall(handle.resize_to, handle, 0)
            local huge_ok = pcall(handle.resize_to, handle, 41)
            neotmux.on_tick(function(ctx)
              ctx:text(nil, 0, 0, tostring(shut_ok) .. '/' .. tostring(huge_ok)
                .. ': ' .. tostring(shut_err))
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    let reported = texts(&sub);
    assert!(
        reported[0].starts_with("false/false: ") && reported[0].contains("at least one"),
        "{reported:?}"
    );
    assert_eq!(sub.reserved(), Reserved::vertical(10, 0));
}

/// The bug this answers: a plugin drawing into the bottom-right corner put its box where
/// `ctx:size()` said the right edge was, which is the pane area's *size* — so every column a
/// sidebar reserved on the left moved the box that far off the corner. `ctx:area()` carries
/// the origin the size leaves out.
#[test]
fn the_area_a_tick_is_given_is_placed_where_the_bands_left_it() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.rail')"),
        (
            "lua/plugin/rail.lua",
            r#"
            neotmux.reserve('left', 12)
            neotmux.on_tick(function(ctx)
              local a = ctx:area()
              local s = ctx:size()
              ctx:text(nil, 0, 0, a.x .. ',' .. a.y .. ' ' .. a.w .. 'x' .. a.h
                .. ' size=' .. s.cols .. 'x' .. s.rows)
            end)
            return {}
            "#,
        ),
    ]);
    frame(&sub, true);
    assert_eq!(texts(&sub), vec!["12,0 68x24 size=68x24".to_string()]);
}

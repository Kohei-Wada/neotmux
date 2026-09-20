//! `neotmux.reserve` and `ctx:reserved()` across the ABI: what a source is granted, what it
//! is refused, and which band it draws in.

use std::fs;

use neotmux_core::proto::{DrawCmd, Epoch, PaneId, Rect, Reserved, Size};
use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;

const SURFACE: Size = Size { rows: 24, cols: 80 };

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

/// What the host does at a frame boundary: settle the surface, hand out the bands, publish
/// the geometry the panes were left with, and tick.
fn frame(sub: &LuaSubsystem, applied: bool) {
    let reserved = sub.reserved();
    sub.lay_out_reservations(SURFACE, applied);
    let content = Size {
        rows: SURFACE.rows - if applied { reserved.rows() } else { 0 },
        cols: SURFACE.cols,
    };
    let pane = PaneId::from(1);
    let rect = Rect::new(0, if applied { reserved.top } else { 0 }, 80, content.rows);
    sub.begin_frame(
        Epoch::new(content, vec![pane], Some(pane)),
        content,
        vec![(pane, rect)],
        Some(pane),
    );
    sub.dispatch_tick();
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
    format!(
        r#"
        neotmux.reserve('bottom', 1)
        neotmux.on_tick(function(ctx)
          local band = ctx:reserved()
          ctx:text(nil, 0, 0, '{name}=' .. (band and (band.y .. '+' .. band.h) or 'nil'))
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
    assert_eq!(sub.reserved(), Reserved::new(0, 2));
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
        Reserved::new(0, 2),
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
        Reserved::new(0, 4),
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

    sub.dispatch_key("C-r");
    assert_eq!(sub.reserved(), Reserved::new(0, 1));
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec!["b=23+1".to_string()],
        "the survivor did not move out to the edge the removed band held"
    );
}

/// A malformed argument raises, like every other one in the ABI. `left` and `right` are not
/// edges: a band is measured in rows.
#[test]
fn a_vertical_edge_raises() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.sideways')"),
        (
            "lua/plugin/sideways.lua",
            r#"
            local ok, err = pcall(neotmux.reserve, 'left', 1)
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
        reported[0].starts_with("false: ") && reported[0].contains("unknown edge 'left'"),
        "{reported:?}"
    );
    assert_eq!(sub.reserved(), Reserved::default());
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

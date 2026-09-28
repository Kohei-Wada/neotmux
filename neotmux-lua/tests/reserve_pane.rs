//! `neotmux.reserve_pane` across the ABI: what a source is granted, what it is refused, and
//! which columns of a pane it draws in.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size};

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

/// What the host publishes for one pane over the whole surface, `band` columns of it given up
/// on the left — or, with `band` 0, a pane that did not get its bands this frame.
fn frame(sub: &LuaSubsystem, band: u16) {
    let pane = PaneId::from(1);
    let outer = Rect::new(0, 0, SURFACE.cols, SURFACE.rows);
    let content = Rect::new(band, 0, SURFACE.cols - band, SURFACE.rows);
    sub.begin_frame(
        support::one(),
        Epoch::new(SURFACE, vec![pane], Some(pane)),
        Geometry {
            area: outer,
            panes: vec![(pane, content)],
            outer: vec![(pane, outer)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());
}

fn texts(sub: &LuaSubsystem) -> Vec<String> {
    sub.frame_output()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A plugin that takes `cells` of every pane and reports where its band in pane 1 landed.
fn reporter(name: &str, cells: u16) -> String {
    format!(
        r#"
        neotmux.reserve_pane('left', {cells})
        neotmux.on_tick(function(ctx)
          local band = ctx:pane_band(1)
          ctx:text(nil, 0, 0, '{name}=' .. (band and (band.x .. '+' .. band.w .. 'x' .. band.h) or 'nil'))
        end)
        return {{}}
        "#
    )
}

#[test]
fn two_sources_get_their_bands_side_by_side_from_the_pane_edge_in() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", &reporter("a", 3)),
        ("lua/plugin/b.lua", &reporter("b", 4)),
    ]);
    assert_eq!(sub.pane_band(), 7);
    frame(&sub, 7);
    assert_eq!(
        texts(&sub),
        vec!["a=0+3x24".to_string(), "b=3+4x24".to_string()]
    );
}

#[test]
fn a_pane_that_did_not_get_its_bands_has_none_to_draw_in() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a')"),
        ("lua/plugin/a.lua", &reporter("a", 3)),
    ]);
    frame(&sub, 0);
    assert_eq!(texts(&sub), vec!["a=nil".to_string()]);
}

/// A band asked for inside a tick is not drawn until the host has narrowed the pane for it.
#[test]
fn a_band_the_pane_has_not_made_room_for_yet_is_not_handed_out() {
    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        ("lua/plugin/a.lua", &reporter("a", 3)),
        ("lua/plugin/b.lua", &reporter("b", 4)),
    ]);
    frame(&sub, 3);
    assert_eq!(
        texts(&sub),
        vec!["a=0+3x24".to_string(), "b=nil".to_string()]
    );
}

#[test]
fn the_outer_rect_is_the_pane_with_its_bands_and_falls_back_to_the_content() {
    let (_dir, sub) = built(&[(
        "init.lua",
        r#"
        neotmux.on_tick(function(ctx)
          local r = ctx:pane_outer(1)
          ctx:text(nil, 0, 0, r.x .. '+' .. r.w)
          ctx:text(nil, 0, 1, tostring(ctx:pane_outer(99)))
        end)
        "#,
    )]);
    frame(&sub, 5);
    assert_eq!(texts(&sub), vec!["0+80".to_string(), "nil".to_string()]);

    let pane = PaneId::from(1);
    let content = Rect::new(5, 0, 75, 24);
    sub.begin_frame(
        support::one(),
        Epoch::new(SURFACE, vec![pane], Some(pane)),
        Geometry {
            area: content,
            panes: vec![(pane, content)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(support::one());
    assert_eq!(texts(&sub), vec!["5+75".to_string(), "nil".to_string()]);
}

#[test]
fn a_second_band_from_one_source_and_one_past_the_cap_are_refused_with_a_message() {
    let (_dir, sub) = built(&[(
        "init.lua",
        r#"
        local first = neotmux.reserve_pane('left', 10)
        local second, why = neotmux.reserve_pane('left', 1)
        assert(first and second == nil and why:find('already holds'), why)
        assert(first:resize_to(16))
        assert(not pcall(first.resize_to, first, 17), 'a count past the cap raises')
        "#,
    )]);
    assert_eq!(sub.pane_band(), 16);

    let (_dir, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        (
            "lua/plugin/a.lua",
            "assert(neotmux.reserve_pane('left', 10)) return {}",
        ),
        (
            "lua/plugin/b.lua",
            "local h, why = neotmux.reserve_pane('left', 7)\n\
             assert(h == nil and why:find('will not fit'), why)\n\
             return {}",
        ),
    ]);
    assert_eq!(sub.pane_band(), 10);
}

#[test]
fn an_edge_other_than_left_and_a_count_out_of_range_raise() {
    for body in [
        "neotmux.reserve_pane('top', 1)",
        "neotmux.reserve_pane('left', 0)",
        "neotmux.reserve_pane('left', 17)",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("init.lua"), body).unwrap();
        let sub = LuaSubsystem::build(
            Layers::from_paths(vec![dir.path().to_path_buf()]),
            support::panes(),
        )
        .unwrap();
        assert!(!sub.run_init().is_empty(), "{body} did not raise");
        assert_eq!(sub.pane_band(), 0);
    }
}

#[test]
fn removing_the_handle_or_emptying_its_group_gives_the_columns_back() {
    let (_dir, sub) = built(&[(
        "init.lua",
        r#"
        neotmux.reserve_pane('left', 4):remove()
        local g = neotmux.group('g')
        g.reserve_pane('left', 6)
        neotmux.group('g')
        "#,
    )]);
    assert_eq!(sub.pane_band(), 0);
}

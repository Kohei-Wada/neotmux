//! A band or a float showing a buffer: the host draws it every frame with no Lua running.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{Color, DrawCmd, Epoch, Geometry, PaneId, Rect, Reserved, SessionId, Size};

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
    frame_in(sub, support::one(), applied)
}

fn frame_in(sub: &LuaSubsystem, session: SessionId, applied: bool) {
    let reserved = if applied {
        sub.reserved()
    } else {
        Reserved::default()
    };
    sub.lay_out_reservations(session, SURFACE, reserved);
    let content = Size {
        rows: SURFACE.rows - reserved.rows(),
        cols: SURFACE.cols - reserved.cols(),
    };
    let pane = PaneId::from(1);
    let rect = Rect::new(reserved.left, reserved.top, content.cols, content.rows);
    sub.begin_frame(
        session,
        Epoch::new(content, vec![pane], Some(pane)),
        Geometry {
            area: rect,
            panes: vec![(pane, rect)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    sub.dispatch_tick(session);
}

fn texts(sub: &LuaSubsystem) -> Vec<(u16, u16, String)> {
    texts_of(sub.frame_output())
}

/// What `session` is shown.
fn texts_in(sub: &LuaSubsystem, session: SessionId) -> Vec<(u16, u16, String)> {
    texts_of(sub.frame_over(session, &sub.frame_immediate()))
}

fn texts_of(cmds: Vec<DrawCmd>) -> Vec<(u16, u16, String)> {
    cmds.into_iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { x, y, text, .. } => Some((x, y, text)),
            _ => None,
        })
        .collect()
}

const BAND: &str = "local buf = neotmux.buf.create()\n\
    neotmux.buf.set_lines(buf, 0, -1, { 'one', 'two' })\n\
    local band = neotmux.reserve('left', 6)\n\
    band:set_buf(buf)\n\
    return { buf = buf, band = band }";

#[test]
fn a_band_shows_its_buffers_lines_from_its_top_row() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        ("lua/plugin/b.lua", BAND),
    ]);
    frame(&sub, true);
    assert_eq!(
        texts(&sub),
        vec![(0, 0, "one".to_string()), (0, 1, "two".to_string())]
    );
}

#[test]
fn nothing_is_drawn_without_a_tick_source() {
    // The plugin registers no on_tick at all, and the band still shows the lines.
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        ("lua/plugin/b.lua", BAND),
    ]);
    frame(&sub, true);
    assert!(!texts(&sub).is_empty());
}

#[test]
fn a_line_wider_than_the_window_is_cut_at_it() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "local buf = neotmux.buf.create()\n\
             neotmux.buf.set_lines(buf, 0, -1, { 'abcdefghij' })\n\
             neotmux.reserve('left', 4):set_buf(buf)\n\
             return {}",
        ),
    ]);
    frame(&sub, true);
    assert_eq!(texts(&sub), vec![(0, 0, "abcd".to_string())]);
}

#[test]
fn a_wide_character_at_the_windows_edge_is_left_out() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "local buf = neotmux.buf.create()\n\
             neotmux.buf.set_lines(buf, 0, -1, { 'abc\u{65e5}' })\n\
             neotmux.reserve('left', 4):set_buf(buf)\n\
             return {}",
        ),
    ]);
    frame(&sub, true);
    assert_eq!(texts(&sub), vec![(0, 0, "abc".to_string())]);
}

#[test]
fn extmarks_colour_their_columns_and_a_line_group_paints_the_row() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "neotmux.hl.set('t:hi', { fg = '#ff0000', bold = true })\n\
             neotmux.hl.set('t:row', { bg = '#000011' })\n\
             local buf = neotmux.buf.create()\n\
             local ns = neotmux.buf.namespace('t')\n\
             neotmux.buf.set_lines(buf, 0, -1, { 'a\u{65e5}bc' })\n\
             neotmux.buf.set_extmark(buf, ns, 0, 1, { end_col = 3, hl_group = 't:hi' })\n\
             neotmux.buf.set_extmark(buf, ns, 0, 0, { line_hl_group = 't:row' })\n\
             neotmux.reserve('left', 8):set_buf(buf)\n\
             return {}",
        ),
    ]);
    frame(&sub, true);
    let out = sub.frame_output();
    let row_bg = Color::Rgb(0, 0, 0x11);
    assert!(
        out.iter().any(|c| matches!(c,
        DrawCmd::Rect { rect, bg, .. } if *rect == Rect::new(0, 0, 8, 1) && *bg == row_bg)),
        "{out:?}"
    );
    assert!(
        out.iter().any(|c| matches!(c,
        DrawCmd::Text { x: 1, text, fg, bold: true, bg, .. }
            if text == "\u{65e5}" && *fg == Color::Rgb(0xff, 0, 0) && *bg == row_bg)),
        "{out:?}"
    );
    assert!(
        out.iter().any(|c| matches!(c,
        DrawCmd::Text { x: 3, text, fg: Color::Default, bold: false, .. } if text == "bc")),
        "{out:?}"
    );
}

#[test]
fn a_line_group_with_only_a_foreground_paints_the_row_with_it() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "neotmux.hl.set('t:row', { fg = '#000022' })\n\
             local buf = neotmux.buf.create()\n\
             local ns = neotmux.buf.namespace('t')\n\
             neotmux.buf.set_lines(buf, 0, -1, { 'a' })\n\
             neotmux.buf.set_extmark(buf, ns, 0, 0, { line_hl_group = 't:row' })\n\
             neotmux.reserve('left', 8):set_buf(buf)\n\
             return {}",
        ),
    ]);
    frame(&sub, true);
    let out = sub.frame_output();
    assert!(
        out.iter().any(|c| matches!(c,
        DrawCmd::Rect { rect, bg, .. }
            if *rect == Rect::new(0, 0, 8, 1) && *bg == Color::Rgb(0, 0, 0x22))),
        "{out:?}"
    );
}

#[test]
fn immediate_drawing_goes_over_the_buffer() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "local buf = neotmux.buf.create()\n\
             neotmux.buf.set_lines(buf, 0, -1, { 'under' })\n\
             neotmux.reserve('left', 6):set_buf(buf)\n\
             neotmux.on_tick(function(ctx) local b = ctx:reserved() if b then ctx:text(nil, b.x, b.y, 'OVER') end end)\n\
             return {}",
        ),
    ]);
    frame(&sub, true);
    let order: Vec<String> = texts(&sub).into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(order, vec!["under".to_string(), "OVER".to_string()]);
}

#[test]
fn a_removed_window_shows_nothing() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "local buf = neotmux.buf.create()\n\
             neotmux.buf.set_lines(buf, 0, -1, { 'x' })\n\
             local band = neotmux.reserve('left', 6)\n\
             band:set_buf(buf)\n\
             band:remove()\n\
             local win = neotmux.float({ x = 0, y = 0, w = 4, h = 1 })\n\
             win:set_buf(buf)\n\
             win:remove()\n\
             return {}",
        ),
    ]);
    frame(&sub, true);
    assert!(texts(&sub).is_empty(), "{:?}", texts(&sub));
}

#[test]
fn a_float_shows_a_buffer_at_its_own_origin() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "local buf = neotmux.buf.create()\n\
             neotmux.buf.set_lines(buf, 0, -1, { 'hi' })\n\
             neotmux.float({ x = 10, y = 5, w = 4, h = 2 }):set_buf(buf)\n\
             return {}",
        ),
    ]);
    frame(&sub, true);
    assert_eq!(texts(&sub), vec![(10, 5, "hi".to_string())]);
}

#[test]
fn a_geometry_change_keeps_the_buffer_where_immediate_drawing_would_be_lost() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        ("lua/plugin/b.lua", BAND),
    ]);
    frame(&sub, true);
    // A new epoch with no tick: immediate drawing from the last one would not apply.
    let pane = PaneId::from(1);
    let rect = Rect::new(6, 0, 70, 24);
    sub.begin_frame(
        support::one(),
        Epoch::new(Size { rows: 24, cols: 70 }, vec![pane], Some(pane)),
        Geometry {
            area: rect,
            panes: vec![(pane, rect)],
            focus: Some(pane),
            ..Default::default()
        },
    );
    assert_eq!(texts(&sub).len(), 2);
}

#[test]
fn set_buf_on_a_deleted_buffer_raises() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        (
            "lua/plugin/b.lua",
            "local buf = neotmux.buf.create()\n\
             neotmux.buf.delete(buf)\n\
             local ok, err = pcall(function() neotmux.reserve('left', 4):set_buf(buf) end)\n\
             neotmux.log.info('set_buf ' .. tostring(ok) .. ' ' .. tostring(err))\n\
             return {}",
        ),
    ]);
    let ops = format!("{:?}", sub.drain_ops(None));
    assert!(ops.contains("set_buf false"), "{ops}");
    assert!(ops.contains("does not exist"), "{ops}");
}

/// Two sessions and one band: what `set_buf` was given at load is every session's, and what it is
/// given in a session's tick is that session's alone, down to showing nothing, until it ends.
const PER_SESSION: &str = "local default = neotmux.buf.create()\n\
    neotmux.buf.set_lines(default, 0, -1, { 'dflt' })\n\
    local mine = neotmux.buf.create()\n\
    neotmux.buf.set_lines(mine, 0, -1, { 'two' })\n\
    local band = neotmux.reserve('left', 6)\n\
    band:set_buf(default)\n\
    neotmux.on_tick(function()\n\
      local who = neotmux.session.current()\n\
      if who == 'two' then band:set_buf(mine) elseif who == 'three' then band:set_buf(nil) end\n\
    end)\n\
    return {}";

fn sessions(sub: &LuaSubsystem) -> [SessionId; 3] {
    let ids = [SessionId::from(1), SessionId::from(2), SessionId::from(3)];
    for (id, name) in ids.iter().zip(["one", "two", "three"]) {
        sub.name_session(*id, name);
    }
    for id in ids {
        frame_in(sub, id, true);
    }
    ids
}

#[test]
fn a_buffer_given_in_a_sessions_tick_is_shown_in_that_session_only() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        ("lua/plugin/b.lua", PER_SESSION),
    ]);
    let [one, two, _] = sessions(&sub);
    assert_eq!(texts_in(&sub, one), vec![(0, 0, "dflt".to_string())]);
    assert_eq!(texts_in(&sub, two), vec![(0, 0, "two".to_string())]);
    // One ticking after two changes nothing two is shown.
    frame_in(&sub, one, true);
    assert_eq!(texts_in(&sub, two), vec![(0, 0, "two".to_string())]);
}

#[test]
fn set_buf_nil_in_a_sessions_tick_shows_nothing_there_only() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        ("lua/plugin/b.lua", PER_SESSION),
    ]);
    let [one, _, three] = sessions(&sub);
    assert!(
        texts_in(&sub, three).is_empty(),
        "{:?}",
        texts_in(&sub, three)
    );
    assert_eq!(texts_in(&sub, one), vec![(0, 0, "dflt".to_string())]);
}

#[test]
fn a_session_that_ends_takes_its_own_buffer_with_it() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.b')"),
        ("lua/plugin/b.lua", PER_SESSION),
    ]);
    let [_, two, _] = sessions(&sub);
    sub.forget_session(two);
    assert_eq!(texts_in(&sub, two), vec![(0, 0, "dflt".to_string())]);
}

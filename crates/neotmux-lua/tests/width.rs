//! `neotmux.width`: how many columns a string takes, read from Lua.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;

/// `body` runs on every tick.
fn built(body: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    let init = format!("neotmux.on_tick(function()\n{body}\nend)\n");
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn ticked(sub: &LuaSubsystem) -> Vec<String> {
    sub.dispatch_tick();
    sub.drain_ops()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_wide_character_is_two_columns_where_lua_would_count_three_bytes() {
    let (_dir, sub) = built(
        "local s = 'a\u{3042}b'\n\
         neotmux.log.info(neotmux.width(s) .. ' of ' .. #s)",
    );
    assert_eq!(ticked(&sub), vec!["4 of 5"]);
}

#[test]
fn ascii_is_one_column_a_character() {
    let (_dir, sub) = built("neotmux.log.info(neotmux.width('hello'))");
    assert_eq!(ticked(&sub), vec!["5"]);
}

#[test]
fn an_empty_string_is_no_columns_at_all() {
    let (_dir, sub) = built("neotmux.log.info(neotmux.width(''))");
    assert_eq!(ticked(&sub), vec!["0"]);
}

/// The API is sealed, so a plugin cannot replace the measurement everything else agrees on.
#[test]
fn width_cannot_be_replaced() {
    let (_dir, sub) = built(
        "local ok, err = pcall(function() neotmux.width = function() return 0 end end)\n\
         neotmux.log.info(tostring(ok))",
    );
    assert_eq!(ticked(&sub), vec!["false"]);
}

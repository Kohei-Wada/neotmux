//! `neotmux.session_name()`: which session the work in hand is for. One Lua layer serves all
//! of them, so the answer is a property of the dispatch rather than of the layer.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;
use neotmux_proto::SessionId;

mod support;

fn named(init: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn logged(sub: &LuaSubsystem, session: Option<SessionId>) -> Vec<String> {
    sub.drain_ops(session)
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::ops::Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A tick is told the name of the session it is for, and two sessions are told two names by
/// the one layer.
#[test]
fn a_tick_is_told_the_name_of_the_session_it_is_for() {
    let (_dir, sub) = named(
        "neotmux.on_tick(function() neotmux.log.info('name=' .. neotmux.session_name()) end)",
    );
    let work = support::one();
    let home = SessionId::from(2);
    sub.name_session(work, "work");
    sub.name_session(home, "home");

    sub.dispatch_tick(work);
    sub.dispatch_tick(home);

    assert_eq!(logged(&sub, Some(work)), vec!["name=work".to_string()]);
    assert_eq!(logged(&sub, Some(home)), vec!["name=home".to_string()]);
}

/// A load runs before any session exists, so there is no name to answer with. `nil` is the
/// honest answer and the one every other per-session fact already gives at load: the geometry
/// is empty there for the same reason.
#[test]
fn a_load_has_no_session_to_name() {
    let (_dir, sub) = named("neotmux.log.info('at load ' .. tostring(neotmux.session_name()))");
    assert_eq!(logged(&sub, None), vec!["at load nil".to_string()]);
}

//! `neotmux.session_name()`: the one fact about which session this client is in, which
//! nothing in the Lua layer could otherwise be told.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;

mod support;

/// A client that was started with the name is told it by the API.
#[test]
fn the_named_client_is_told_its_name() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "neotmux.on_tick(function()\n\
         neotmux.log.info('name=' .. neotmux.session_name())\n\
         end)\n",
    )
    .unwrap();
    let sub = LuaSubsystem::build_named(
        "work",
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
        std::rc::Rc::new(neotmux_lua::clipboard::Absent),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.dispatch_tick(support::one());
    let line = sub
        .drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::ops::Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .find(|t| t.starts_with("name="))
        .expect("the tick logged the name");
    assert_eq!(line, "name=work");
}

#[test]
fn a_client_with_no_name_is_told_the_default() {
    let dir = tempfile::tempdir().unwrap();
    // A name that is not the default raises on load, so a clean load is the assertion:
    // the init body checks the name, and `run_init` is where a raise would surface.
    fs::write(
        dir.path().join("init.lua"),
        "if neotmux.session_name() ~= 'default' then\n  error('named: ' .. neotmux.session_name())\nend\n",
    )
    .unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(
        sub.run_init().is_empty(),
        "the layer loads only if the name is the default"
    );
}

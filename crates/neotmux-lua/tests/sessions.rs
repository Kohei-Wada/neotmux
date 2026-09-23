//! `neotmux.session.*`: the sessions a plugin can see, and the one it is running for.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, SessionId, Size};

mod support;

fn built(init: &str) -> (tempfile::TempDir, LuaSubsystem) {
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

/// A session the host has made and told the layer about, with panes in it.
fn make(sub: &LuaSubsystem, id: SessionId, name: &str, panes: u32) {
    sub.name_session(id, name);
    let size = Size { rows: 24, cols: 80 };
    let panes: Vec<(PaneId, Rect)> = (1..=panes)
        .map(|n| (PaneId::from(n), Rect::new(0, 0, 80, 24)))
        .collect();
    let focus = panes.first().map(|(id, _)| *id);
    sub.begin_frame(
        id,
        Epoch::new(size, panes.iter().map(|(id, _)| *id).collect(), focus),
        Geometry {
            area: Rect::new(0, 0, size.cols, size.rows),
            panes,
            focus,
            ..Default::default()
        },
    );
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

#[test]
fn a_plugin_sees_every_session_and_which_one_it_is_running_for() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 neotmux.log.info(neotmux.session.current() .. ' of ' .. table.concat(neotmux.session.list(), ','))\n\
         end)",
    );
    let work = support::one();
    let home = SessionId::from(2);
    make(&sub, work, "work", 1);
    make(&sub, home, "home", 1);

    sub.dispatch_tick(work);
    sub.dispatch_tick(home);

    assert_eq!(
        logged(&sub, Some(work)),
        vec!["work of home,work".to_string()]
    );
    assert_eq!(
        logged(&sub, Some(home)),
        vec!["home of home,work".to_string()]
    );
}

/// A load has no session, so there is nothing to be current and nothing has been named yet.
#[test]
fn a_load_sees_no_sessions_at_all() {
    let (_dir, sub) = built(
        "neotmux.log.info(tostring(neotmux.session.current()) .. ' of ' .. #neotmux.session.list())",
    );
    assert_eq!(logged(&sub, None), vec!["nil of 0".to_string()]);
}

/// The whole point: a plugin reads a session it is not running for, with every ordinary read,
/// and is put back where it was afterwards.
#[test]
fn with_reads_another_session_and_puts_the_caller_back() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 local theirs = neotmux.session.with('home', function() return #neotmux.pane.list() end)\n\
         \x20 neotmux.log.info('mine=' .. #neotmux.pane.list() .. ' theirs=' .. tostring(theirs)\n\
         \x20   .. ' back=' .. neotmux.session.current())\n\
         end)",
    );
    let work = support::one();
    let home = SessionId::from(2);
    make(&sub, work, "work", 1);
    make(&sub, home, "home", 3);

    sub.dispatch_tick(work);

    assert_eq!(
        logged(&sub, Some(work)),
        vec!["mine=1 theirs=3 back=work".to_string()]
    );
}

/// A name nothing answers to is not an error — a session can end between one tick and the
/// next. The function does not run.
#[test]
fn with_a_name_nothing_answers_to_does_nothing() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 local ran = false\n\
         \x20 local got = neotmux.session.with('gone', function() ran = true return 1 end)\n\
         \x20 neotmux.log.info('got=' .. tostring(got) .. ' ran=' .. tostring(ran))\n\
         end)",
    );
    make(&sub, support::one(), "work", 1);
    sub.dispatch_tick(support::one());
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["got=nil ran=false".to_string()]
    );
}

/// An error inside the call is the caller's error, and the binding is undone either way: the
/// guard is what restores, so a raise cannot leave the layer reading somebody else's session.
#[test]
fn with_puts_the_caller_back_even_when_the_call_raises() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 pcall(function() neotmux.session.with('home', function() error('nope') end) end)\n\
         \x20 neotmux.log.info('back=' .. neotmux.session.current())\n\
         end)",
    );
    make(&sub, support::one(), "work", 1);
    make(&sub, SessionId::from(2), "home", 1);
    sub.dispatch_tick(support::one());
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["back=work".to_string()]
    );
}

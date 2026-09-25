//! `neotmux.group`: a named set of registrations that empties itself when it is named again.

use std::fs;
use std::rc::Rc;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::{JobChunk, Op};
use neotmux_lua::plugin::{KindTag, Layers};

mod support;

fn built(
    files: &[(&str, &str)],
    runner: Rc<dyn neotmux_lua::host::Runner>,
) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
        Rc::new(neotmux_lua::host::Absent),
        runner,
    )
    .unwrap();
    let failures = sub.run_init();
    assert!(failures.is_empty(), "{failures:?}");
    (dir, sub)
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(None)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn ticks(sub: &LuaSubsystem) -> usize {
    sub.registry().snapshot(KindTag::Tick).len()
}

#[test]
fn naming_a_group_again_empties_it() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "local g = neotmux.group('t')
             g.on_tick(function() end)
             g.on_event('resized', function() end)
             g.keymap.set('C-x', function() return true end)
             g.command.add('scratch', function() end)
             assert(g.reserve('bottom', 1))
             assert(g.float({ x = 0, y = 0, w = 1, h = 1 }))
             g.timer.every(1000, function() end)
             neotmux.group('t')",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert_eq!(ticks(&sub), 0);
    assert!(sub.registry().snapshot(KindTag::Event).is_empty());
    assert!(sub.registry().snapshot(KindTag::Keymap).is_empty());
    assert!(sub.commands().is_empty());
    assert_eq!(sub.reserved(), Default::default());
    assert_eq!(sub.floats().count(), 0);
    assert_eq!(sub.census().timers, 0);
}

#[test]
fn registrations_outside_the_group_are_left_alone() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "local g = neotmux.group('t')
             g.on_tick(function() end)
             neotmux.on_tick(function() end)
             neotmux.group('t')",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert_eq!(ticks(&sub), 1);
}

/// Names are one namespace: two plugins that pick the same one share a group.
#[test]
fn two_sources_naming_the_same_group_share_it() {
    let (_dir, sub) = built(
        &[
            (
                "lua/plugin/a.lua",
                "neotmux.group('shared').on_tick(function() end)",
            ),
            ("lua/plugin/b.lua", "neotmux.group('shared')"),
            ("init.lua", "require('plugin.a') require('plugin.b')"),
        ],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert_eq!(ticks(&sub), 0);
}

#[test]
fn a_group_call_answers_what_the_plain_call_does_refusals_included() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "local g = neotmux.group('t')
             local h = g.reserve('bottom', 1)
             assert(type(h) == 'table' and type(h.remove) == 'function')
             local none, why = g.reserve('bottom', 4)
             assert(none == nil and type(why) == 'string', 'the cap should refuse')
             local ok = pcall(g.reserve, 'nowhere', 1)
             assert(not ok, 'an unknown edge should raise through the group too')",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert!(logged(&sub).is_empty());
}

#[test]
fn a_name_that_is_not_a_non_empty_string_raises() {
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "assert(not pcall(neotmux.group, ''))
             assert(not pcall(neotmux.group, {}))
             assert(pcall(neotmux.group, 'ok'))",
        )],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    assert!(logged(&sub).is_empty());
}

/// The owner is still the chunk the call was written in, not the host's wrapper.
#[test]
fn a_group_call_is_attributed_to_the_chunk_that_made_it() {
    let (_dir, sub) = built(
        &[
            (
                "lua/plugin/banded.lua",
                "assert(neotmux.group('b').reserve('bottom', 1))",
            ),
            ("init.lua", "require('plugin.banded')"),
        ],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    let owners: Vec<String> = sub
        .reservation_owners()
        .into_iter()
        .filter_map(|id| sub.sources().name(id))
        .collect();
    assert_eq!(owners, vec!["plugin/banded".to_string()]);
}

/// A job in an emptied group is killed, and its `on_exit` — the old code's — is never called.
#[test]
fn a_job_in_an_emptied_group_is_killed_without_its_on_exit() {
    let runner = support::FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"x\n".to_vec())]]);
    let (_dir, sub) = built(
        &[(
            "init.lua",
            "neotmux.group('t').job.start({ 'sleep' }, {
               on_exit = function() neotmux.log.info('on_exit ran') end,
             })
             neotmux.group('t')",
        )],
        runner.clone(),
    );
    assert!(*runner.killed.borrow(), "the job ran on");
    sub.dispatch_jobs(support::one());
    assert!(!logged(&sub).iter().any(|l| l == "on_exit ran"));
}

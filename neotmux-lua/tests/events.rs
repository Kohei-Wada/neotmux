use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{Event, PaneId, Size};

mod support;

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
    (dir, sub)
}

#[test]
fn a_subscriber_sees_the_payload_of_pane_output() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        (
            "lua/plugin/watcher.lua",
            "neotmux.on_event('pane_output', function(e) neotmux.log.info('saw ' .. e.bytes) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(
        support::one(),
        &Event::PaneOutput {
            pane: PaneId::from(1),
            bytes: b"hello".to_vec(),
        },
    );
    let ops = sub.drain_ops(Some(support::one()));
    assert!(format!("{ops:?}").contains("saw hello"), "{ops:?}");
}

#[test]
fn resized_carries_the_new_size() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.sizer')"),
        (
            "lua/plugin/sizer.lua",
            "neotmux.on_event('resized', function(e) neotmux.log.info(e.rows .. 'x' .. e.cols) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(
        support::one(),
        &Event::Resized {
            size: Size {
                rows: 30,
                cols: 100,
            },
        },
    );
    assert!(format!("{:?}", sub.drain_ops(Some(support::one()))).contains("30x100"));
}

#[test]
fn every_subscriber_runs_even_when_one_raises() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.bad') require('plugin.good')"),
        (
            "lua/plugin/bad.lua",
            "neotmux.on_event('resized', function() error('boom') end) return {}",
        ),
        (
            "lua/plugin/good.lua",
            "neotmux.on_event('resized', function() neotmux.log.info('still here') end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(
        support::one(),
        &Event::Resized {
            size: Size { rows: 1, cols: 1 },
        },
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("still here"), "{text}");
    assert!(
        text.contains("boom"),
        "the raising subscriber's error should be reported: {text}"
    );
}

#[test]
fn a_subscriber_to_another_event_is_not_called() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.picky')"),
        (
            "lua/plugin/picky.lua",
            "neotmux.on_event('pane_output', function() neotmux.log.info('output') end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(
        support::one(),
        &Event::Resized {
            size: Size { rows: 1, cols: 1 },
        },
    );
    assert!(sub.drain_ops(Some(support::one())).is_empty());
}

#[test]
fn a_subscriber_mutating_the_payload_does_not_affect_the_next_subscriber() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.mutator') require('plugin.observer')",
        ),
        (
            "lua/plugin/mutator.lua",
            "neotmux.on_event('pane_output', function(e) e.bytes = 'processed' e.pane = nil end) return {}",
        ),
        (
            "lua/plugin/observer.lua",
            "neotmux.on_event('pane_output', function(e) neotmux.log.info('saw ' .. e.bytes .. ' pane=' .. tostring(e.pane)) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(
        support::one(),
        &Event::PaneOutput {
            pane: PaneId::from(1),
            bytes: b"hello".to_vec(),
        },
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("saw hello pane=1"), "{text}");
}

/// `unload` has no fields and reaches every subscriber once.
#[test]
fn unload_reaches_every_subscriber_with_an_empty_table() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        (
            "lua/plugin/watcher.lua",
            "neotmux.on_event('unload', function(e) neotmux.log.info('a ' .. tostring(next(e))) end)\n\
             neotmux.on_event('unload', function() neotmux.log.info('b') end)\n\
             return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_unload(Some(support::one()));
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("a nil"), "{text}");
    assert!(text.contains("b"), "{text}");
}

#[test]
fn subscribers_run_in_registration_order() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.first') require('plugin.second')",
        ),
        (
            "lua/plugin/first.lua",
            "neotmux.on_event('resized', function() neotmux.log.info('first') end) return {}",
        ),
        (
            "lua/plugin/second.lua",
            "neotmux.on_event('resized', function() neotmux.log.info('second') end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(
        support::one(),
        &Event::Resized {
            size: Size { rows: 1, cols: 1 },
        },
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    let first_idx = text.find("first").expect("first should have run");
    let second_idx = text.find("second").expect("second should have run");
    assert!(first_idx < second_idx, "{text}");
}

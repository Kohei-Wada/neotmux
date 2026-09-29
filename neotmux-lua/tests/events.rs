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

fn saw(files_body: &str, event: &Event) -> String {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        ("lua/plugin/watcher.lua", files_body),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event(support::one(), event);
    format!("{:?}", sub.drain_ops(Some(support::one())))
}

#[test]
fn pane_changed_carries_the_pane_and_no_bytes() {
    let ops = saw(
        "neotmux.on_event('pane_changed', function(e) neotmux.log.info('changed ' .. e.pane .. ' ' .. tostring(e.bytes)) end) return {}",
        &Event::PaneChanged {
            pane: PaneId::from(7),
        },
    );
    assert!(ops.contains("changed 7 nil"), "{ops}");
}

#[test]
fn window_renamed_carries_the_window() {
    let ops = saw(
        "neotmux.on_event('window_renamed', function(e) neotmux.log.info('renamed ' .. e.window) end) return {}",
        &Event::WindowRenamed {
            window: neotmux_proto::WindowId::from(3),
        },
    );
    assert!(ops.contains("renamed 3"), "{ops}");
}

#[test]
fn session_renamed_carries_both_names() {
    let ops = saw(
        "neotmux.on_event('session_renamed', function(e) neotmux.log.info(e.from .. '->' .. e.session) end) return {}",
        &Event::SessionRenamed {
            session: "work".into(),
            from: "0".into(),
        },
    );
    assert!(ops.contains("0->work"), "{ops}");
}

#[test]
fn session_closed_is_dispatched_with_no_session_bound() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        (
            "lua/plugin/watcher.lua",
            "neotmux.on_event('session_closed', function(e) neotmux.log.info('closed ' .. e.session) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.dispatch_event_unbound(&Event::SessionClosed {
        session: "work".into(),
    });
    let ops = format!("{:?}", sub.drain_ops(None));
    assert!(ops.contains("closed work"), "{ops}");
}

fn running(
    commands: &[(u32, &str)],
) -> std::collections::BTreeMap<PaneId, neotmux_proto::PaneInfo> {
    commands
        .iter()
        .map(|(pane, command)| {
            (
                PaneId::from(*pane),
                neotmux_proto::PaneInfo {
                    command: Some(command.to_string()),
                    cwd: None,
                    job: None,
                },
            )
        })
        .collect()
}

#[test]
fn pane_info_changed_names_only_the_pane_whose_info_moved_and_sees_the_new_value() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        (
            "lua/plugin/watcher.lua",
            "neotmux.on_event('pane_info_changed', function(e) neotmux.log.info('info ' .. e.pane .. ' ' .. neotmux.pane.info(e.pane).command) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    sub.publish_pane_info(support::one(), running(&[(1, "zsh"), (2, "zsh")]));
    sub.drain_ops(Some(support::one()));

    sub.publish_pane_info(support::one(), running(&[(1, "zsh"), (2, "vim")]));
    let ops = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert_eq!(ops.matches("info ").count(), 1, "{ops}");
    assert!(ops.contains("info 2 vim"), "{ops}");

    sub.publish_pane_info(support::one(), running(&[(1, "zsh"), (2, "vim")]));
    let ops = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(!ops.contains("info "), "{ops}");
}

#[test]
fn pane_info_changed_compares_against_the_same_sessions_last_map() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        (
            "lua/plugin/watcher.lua",
            "neotmux.on_event('pane_info_changed', function(e) neotmux.log.info('info ' .. e.pane) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    let two = neotmux_proto::SessionId::from(2);
    sub.publish_pane_info(support::one(), running(&[(1, "zsh")]));
    sub.publish_pane_info(two, running(&[(1, "vim")]));
    sub.drain_ops(Some(support::one()));
    sub.drain_ops(Some(two));

    sub.publish_pane_info(support::one(), running(&[(1, "zsh")]));
    let ops = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(!ops.contains("info "), "{ops}");
}

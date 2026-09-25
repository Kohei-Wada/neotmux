use std::fs;

use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};

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
fn returning_true_consumes_the_key() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.binder')"),
        (
            "lua/plugin/binder.lua",
            "neotmux.keymap.set('C-x', function() return true end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
}

#[test]
fn returning_false_or_nil_falls_through() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.binder')"),
        (
            "lua/plugin/binder.lua",
            r#"
            neotmux.keymap.set('C-x', function() return false end)
            neotmux.keymap.set('C-y', function() end)
            return {}
            "#,
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Fallthrough
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "C-y"),
        KeyOutcome::Fallthrough
    );
}

#[test]
fn an_unbound_key_falls_through() {
    let (_d, sub) = built(&[("init.lua", "")]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-z"),
        KeyOutcome::Fallthrough
    );
}

#[test]
fn key_specs_are_normalised_for_conflict_purposes() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.a') require('plugin.b')"),
        (
            "lua/plugin/a.lua",
            "neotmux.keymap.set('C-x', function() return true end) return {}",
        ),
        (
            "lua/plugin/b.lua",
            "local h, err = neotmux.keymap.set('ctrl-x', function() return true end) if h ~= nil then error('conflict not detected') end return {}",
        ),
    ]);
    assert!(
        sub.run_init().is_empty(),
        "the conflicting registration should not raise"
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
}

#[test]
fn a_conflict_returns_nil_and_a_message_and_logs_both_names() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.first') require('plugin.second')",
        ),
        (
            "lua/plugin/first.lua",
            "neotmux.keymap.set('C-x', function() return true end) return {}",
        ),
        (
            "lua/plugin/second.lua",
            r#"
            local h, err = neotmux.keymap.set('C-x', function() return true end)
            neotmux.log.warn('handle=' .. tostring(h) .. ' err=' .. tostring(err))
            return {}
            "#,
        ),
    ]);
    assert!(sub.run_init().is_empty());
    let ops = sub.drain_ops(None);
    let text = format!("{ops:?}");
    assert!(text.contains("handle=nil"), "{text}");
    assert!(
        text.contains("first"),
        "the conflict message should name the holder: {text}"
    );
}

#[test]
fn the_loser_of_a_conflict_does_not_shadow_the_winner() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.first') require('plugin.second')",
        ),
        (
            "lua/plugin/first.lua",
            "neotmux.keymap.set('C-x', function() return true end) return {}",
        ),
        (
            "lua/plugin/second.lua",
            "neotmux.keymap.set('C-x', function() return false end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
}

#[test]
fn no_key_is_reserved_by_the_host() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.prefixer')"),
        (
            "lua/plugin/prefixer.lua",
            r#"
            local h, err = neotmux.keymap.set('C-b', function() return true end)
            if h == nil then error('C-b refused: ' .. tostring(err)) end
            return {}
            "#,
        ),
    ]);
    let failures = sub.run_init();
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        sub.dispatch_key(support::one(), "C-b"),
        KeyOutcome::Consumed
    );
}

#[test]
fn a_key_the_host_once_reserved_falls_through_when_nothing_binds_it() {
    let (_d, sub) = built(&[("init.lua", "")]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-b"),
        KeyOutcome::Fallthrough
    );
}

#[test]
fn an_unparseable_spec_raises_at_registration() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.bad')"),
        (
            "lua/plugin/bad.lua",
            "neotmux.keymap.set('Ctrl+Shift+F13', function() end) return {}",
        ),
    ]);
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0]
            .message
            .contains("cannot parse key spec 'Ctrl+Shift+F13'"),
        "{}",
        failures[0].message
    );
}

#[test]
fn a_raising_callback_falls_through_rather_than_swallowing_the_key() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.flaky')"),
        (
            "lua/plugin/flaky.lua",
            "neotmux.keymap.set('C-x', function() error('boom') end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Fallthrough
    );
    assert_eq!(
        sub.drain_ops(Some(support::one())).len(),
        1,
        "the error should be reported"
    );
}

#[test]
fn removing_a_binding_frees_the_key_for_another_plugin() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.transient') require('plugin.later')",
        ),
        (
            "lua/plugin/transient.lua",
            "local h = neotmux.keymap.set('C-x', function() return true end) h:remove() return {}",
        ),
        (
            "lua/plugin/later.lua",
            "local h = neotmux.keymap.set('C-x', function() return true end) if h == nil then error('key was not freed') end return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
}

#[test]
fn normalise_canonicalises_the_modifiers_and_leaves_the_key_alone() {
    use neotmux_lua::keymap::normalise;
    assert_eq!(normalise("C-x").unwrap(), "C-x");
    assert_eq!(normalise("Ctrl-x").unwrap(), "C-x");
    assert_eq!(normalise("M-C-x").unwrap(), "C-M-x");
    assert_eq!(normalise("c-X").unwrap(), "C-X");
    assert!(normalise("Ctrl+Shift+F13").is_err());
    assert!(normalise("").is_err());
}

#[test]
fn an_uppercase_key_and_its_lowercase_are_two_bindings_that_each_fire() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.both')"),
        (
            "lua/plugin/both.lua",
            r#"
            neotmux.keymap.set('x', function() neotmux.log.info('lower') return true end)
            neotmux.keymap.set('X', function() neotmux.log.info('upper') return true end)
            return {}
            "#,
        ),
    ]);
    let failures = sub.run_init();
    assert!(failures.is_empty(), "'X' collided with 'x': {failures:?}");

    assert_eq!(sub.dispatch_key(support::one(), "x"), KeyOutcome::Consumed);
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("lower") && !text.contains("upper"), "{text}");

    assert_eq!(sub.dispatch_key(support::one(), "X"), KeyOutcome::Consumed);
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("upper") && !text.contains("lower"), "{text}");
}

#[test]
fn a_modified_uppercase_key_and_its_lowercase_are_also_distinct() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.both')"),
        (
            "lua/plugin/both.lua",
            r#"
            neotmux.keymap.set('C-x', function() neotmux.log.info('lower') return true end)
            neotmux.keymap.set('C-X', function() neotmux.log.info('upper') return true end)
            return {}
            "#,
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-X"),
        KeyOutcome::Consumed
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("upper") && !text.contains("lower"), "{text}");
}

#[test]
fn shift_with_a_single_character_is_refused_because_it_could_never_fire() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.shifty')"),
        (
            "lua/plugin/shifty.lua",
            "neotmux.keymap.set('S-x', function() return true end) return {}",
        ),
    ]);
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].message.contains("bind 'X' instead"),
        "the refusal should name the key to use instead: {}",
        failures[0].message
    );
}

#[test]
fn on_key_sees_a_key_nothing_has_bound() {
    let (_d, sub) = built(&[
        ("init.lua", "require('plugin.watcher')"),
        (
            "lua/plugin/watcher.lua",
            "neotmux.on_key(function(key) neotmux.log.info('saw ' .. key) end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "c-X"),
        KeyOutcome::Fallthrough
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(
        text.contains("saw C-X"),
        "on_key should receive the normalised spec: {text}"
    );
}

#[test]
fn an_on_key_that_returns_true_consumes_the_key_before_the_binding_runs() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.eater') require('plugin.bound')",
        ),
        (
            "lua/plugin/eater.lua",
            "neotmux.on_key(function() return true end) return {}",
        ),
        (
            "lua/plugin/bound.lua",
            "neotmux.keymap.set('C-x', function() neotmux.log.info('binding ran') return true end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(
        !text.contains("binding ran"),
        "the keymap lookup ran although on_key consumed the key: {text}"
    );
}

#[test]
fn an_on_key_that_returns_false_falls_through_to_the_binding() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.eater') require('plugin.bound')",
        ),
        (
            "lua/plugin/eater.lua",
            "neotmux.on_key(function() return false end) return {}",
        ),
        (
            "lua/plugin/bound.lua",
            "neotmux.keymap.set('C-x', function() neotmux.log.info('binding ran') return true end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "C-z"),
        KeyOutcome::Fallthrough
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("binding ran"), "{text}");
}

#[test]
fn on_key_callbacks_run_in_registration_order_and_the_first_to_consume_stops_the_rest() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.first') require('plugin.second')",
        ),
        (
            "lua/plugin/first.lua",
            "neotmux.on_key(function() neotmux.log.info('first') return true end) return {}",
        ),
        (
            "lua/plugin/second.lua",
            "neotmux.on_key(function() neotmux.log.info('second') return true end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(text.contains("first"), "{text}");
    assert!(
        !text.contains("second"),
        "a later on_key ran after an earlier one consumed the key: {text}"
    );
}

#[test]
fn a_raising_on_key_is_blamed_by_name_and_does_not_swallow_the_key() {
    let (_d, sub) = built(&[
        (
            "init.lua",
            "require('plugin.flaky') require('plugin.bound')",
        ),
        (
            "lua/plugin/flaky.lua",
            "neotmux.on_key(function() error('boom') end) return {}",
        ),
        (
            "lua/plugin/bound.lua",
            "neotmux.keymap.set('C-x', function() neotmux.log.info('binding ran') return true end) return {}",
        ),
    ]);
    assert!(sub.run_init().is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
    let text = format!("{:?}", sub.drain_ops(Some(support::one())));
    assert!(
        text.contains("boom") && text.contains("plugin/flaky"),
        "{text}"
    );
    assert!(
        text.contains("binding ran"),
        "a raising on_key swallowed the key: {text}"
    );
}

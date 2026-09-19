use std::time::Duration;

mod harness;
use harness::Neotmux;

#[test]
fn a_status_provider_reaches_the_screen() {
    let dir = harness::runtime(&[
        ("init.lua", "require('plugin.clock')"),
        (
            "lua/plugin/clock.lua",
            "neotmux.on_tick(function(ctx) ctx:status('STATUS-MARKER') end) return {}",
        ),
    ]);
    let mut nt = Neotmux::start(&dir);
    nt.wait_for_screen("STATUS-MARKER", Duration::from_secs(5));
}

#[test]
fn a_plugin_can_draw_over_the_pane() {
    let dir = harness::runtime(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 0, 0, 'OVERLAY') end) return {}",
        ),
    ]);
    let mut nt = Neotmux::start(&dir);
    nt.wait_for_screen("OVERLAY", Duration::from_secs(5));
}

#[test]
fn a_consumed_key_never_reaches_the_shell() {
    let dir = harness::runtime(&[
        ("init.lua", "require('plugin.binder')"),
        (
            "lua/plugin/binder.lua",
            r#"
            neotmux.keymap.set('C-x', function() return true end)
            neotmux.keymap.set('C-y', function() return false end)
            return {}
            "#,
        ),
    ]);
    let mut nt = Neotmux::start_with_shell(&dir, "cat");
    nt.send("\x18"); // C-x, consumed
    nt.send("\x19"); // C-y, falls through
    nt.send("\r");
    let screen = nt.screen_after(Duration::from_secs(2));
    assert!(
        !screen.contains("^X"),
        "the consumed key reached the shell: {screen:?}"
    );
    assert!(
        screen.contains("^Y"),
        "the fallthrough key did not reach the shell: {screen:?}"
    );
}

#[test]
fn nothing_is_written_while_nothing_happens() {
    let dir = harness::runtime(&[("init.lua", "")]);
    let mut nt = Neotmux::start(&dir);
    nt.settle(Duration::from_secs(1));
    assert_eq!(nt.bytes_in(Duration::from_secs(1)), 0);
}

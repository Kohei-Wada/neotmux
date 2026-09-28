//! `neotmux.costs()`: what each source's ticks have cost, counted by the host around every
//! source's whole tick, and `plugin.costs`, the palette's list of them.

mod support;

use support::{built, keys, logged, one};

/// A source that spins in its tick is counted as costing more than one that does little, and
/// both are counted once a tick. The test's own `init.lua` is one source; the shipped status
/// bar, switched back on, is the other.
#[test]
fn every_source_is_counted_once_a_tick_most_expensive_first() {
    let (_dir, sub) = built(
        "require('neotmux.defaults').status = true\n\
         neotmux.on_tick(function()\n\
           local t = os.clock()\n\
           while os.clock() - t < 0.003 do end\n\
         end)\n\
         neotmux.on_key(function(key)\n\
           if key ~= 'J' then return false end\n\
           for _, c in ipairs(neotmux.costs()) do\n\
             if c.source == 'init' or c.source == 'plugin/tmux/status' then\n\
               neotmux.log.info(c.source .. ' ' .. c.ticks .. ' ' .. tostring(c.total_ms >= c.longest_ms))\n\
             end\n\
           end\n\
           return true\n\
         end)\n",
    );
    for _ in 0..3 {
        sub.dispatch_tick(one());
    }
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub),
        vec![
            "init 3 true".to_string(),
            "plugin/tmux/status 3 true".to_string()
        ]
    );
}

/// The list is a command, so the palette has it.
#[test]
fn the_costs_list_is_a_command() {
    let (_dir, sub) = built("");
    assert!(
        sub.commands().into_iter().any(|c| c.name == "costs"),
        "no costs command"
    );
}

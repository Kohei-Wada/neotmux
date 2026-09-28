//! `plugin.tmux` and `plugin.herdr`: each a set of plugins, loaded as one and turned off as one.

mod support;

use support::{built, keys, logged};

/// Which of the six are loaded, one log line each, on `J`.
const SAY_LOADED: &str = "neotmux.on_key(function(key)\n\
      if key ~= 'J' then return false end\n\
      for _, name in ipairs({ 'status', 'prefix', 'copy', 'sessions', 'borders', 'sync' }) do\n\
        neotmux.log.info(name .. ' ' .. tostring(package.loaded['plugin.tmux.' .. name] ~= nil))\n\
      end\n\
      return true\n\
    end)\n";

/// `defaults.tmux = false` keeps all six out, even with a plugin loaded that binds a prefix key
/// of its own — the palette and hints would otherwise have pulled the prefix in for it.
#[test]
fn turning_tmux_off_loads_none_of_the_six() {
    let (_dir, sub) = built(&format!(
        "local defaults = require('neotmux.defaults')\n\
         defaults.tmux = false\n\
         defaults.status, defaults.prefix, defaults.copy = true, true, true\n\
         defaults.palette, defaults.hints = true, true\n\
         {SAY_LOADED}"
    ));
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub),
        ["status", "prefix", "copy", "sessions", "borders", "sync"]
            .map(|name| format!("{name} false"))
            .to_vec()
    );
}

/// On, each of the six is its own field still: the status bar off, the rest of what is on loads.
#[test]
fn with_tmux_on_each_of_the_six_is_its_own_field() {
    let (_dir, sub) = built(&format!(
        "local defaults = require('neotmux.defaults')\n\
         defaults.tmux = true\n\
         defaults.status, defaults.prefix, defaults.copy = false, true, true\n\
         {SAY_LOADED}"
    ));
    keys(&sub, &["J"]);
    let said = logged(&sub);
    assert!(said.contains(&"status false".to_string()), "{said:?}");
    assert!(said.contains(&"prefix true".to_string()), "{said:?}");
    assert!(said.contains(&"copy true".to_string()), "{said:?}");
}

/// Which of the herdr three are loaded, one log line each, on `J`.
const SAY_HERDR: &str = "neotmux.on_key(function(key)\n\
      if key ~= 'J' then return false end\n\
      for _, name in ipairs({ 'sidebar', 'chime', 'watch' }) do\n\
        neotmux.log.info(name .. ' ' .. tostring(package.loaded['plugin.herdr.' .. name] ~= nil))\n\
      end\n\
      return true\n\
    end)\n";

#[test]
fn turning_herdr_off_loads_none_of_the_three() {
    let (_dir, sub) = built(&format!(
        "local defaults = require('neotmux.defaults')\n\
         defaults.herdr = false\n\
         defaults.sidebar, defaults.chime, defaults.watch = true, true, true\n\
         {SAY_HERDR}"
    ));
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub),
        ["sidebar", "chime", "watch"]
            .map(|name| format!("{name} false"))
            .to_vec()
    );
}

#[test]
fn with_herdr_on_each_of_the_three_is_its_own_field() {
    let (_dir, sub) = built(&format!(
        "local defaults = require('neotmux.defaults')\n\
         defaults.herdr = true\n\
         defaults.sidebar, defaults.chime, defaults.watch = true, false, true\n\
         {SAY_HERDR}"
    ));
    keys(&sub, &["J"]);
    assert_eq!(logged(&sub), ["sidebar true", "chime false", "watch true"]);
}

//! `neotmux.theme`'s shipped themes, and `plugin.theme`'s `theme` command: a list of every one
//! of them, the current one marked, Enter applies it and keeps nothing.

mod support;

use std::rc::Rc;

use support::{built, built_with, keys, logged, notified, one};

const NAMES_26: &str = "
local theme = require('neotmux.theme')
local names = {
  'rosewater', 'flamingo', 'pink', 'mauve', 'red', 'maroon', 'peach', 'yellow', 'green',
  'teal', 'sky', 'sapphire', 'blue', 'lavender', 'text', 'subtext1', 'subtext0', 'overlay2',
  'overlay1', 'overlay0', 'surface2', 'surface1', 'surface0', 'base', 'mantle', 'crust',
}
neotmux.on_key(function(key)
  if key ~= 'L' then return false end
  for _, tname in ipairs(theme.names) do
    theme.use(tname)
    for _, name in ipairs(names) do
      local v = theme.colors[name]
      local ok = type(v) == 'string' and v:match('^#%x%x%x%x%x%x$') ~= nil
      if not ok then
        neotmux.log.info(tname .. '.' .. name .. '=BAD(' .. tostring(v) .. ')')
      end
    end
  end
  neotmux.log.info('checked ' .. #theme.names .. ' themes')
  return true
end)
";

/// Every name in `theme.names` loads and sets all 26 names to a `#rrggbb`.
#[test]
fn every_shipped_theme_sets_all_26_names_to_a_hex_colour() {
    let (_dir, sub) = built(NAMES_26);
    keys(&sub, &["L"]);
    let said = logged(&sub);
    assert_eq!(
        said,
        vec!["checked 13 themes".to_string()],
        "some name was missing or not a #rrggbb: {said:?}"
    );
}

/// `theme.use` changes what every plugin reads through `style.get`: Latte's blue reaches
/// `style.get('status').accent`, which defaults to `'blue'`.
#[test]
fn theme_use_changes_what_style_get_resolves() {
    let (_dir, sub) = built(
        "require('neotmux.defaults').status = true
         local theme = require('neotmux.theme')
         local style = require('neotmux.style')
         neotmux.on_key(function(key)
           if key ~= 'L' then return false end
           theme.use('catppuccin-latte')
           neotmux.log.info('accent=' .. style.get('status').accent)
           return true
         end)",
    );
    keys(&sub, &["L"]);
    assert_eq!(logged(&sub), vec!["accent=#1e66f5"]);
}

/// An unknown theme raises rather than silently doing nothing.
#[test]
fn theme_use_raises_on_an_unknown_name() {
    let (_dir, sub) = built(
        "local theme = require('neotmux.theme')
         neotmux.on_key(function(key)
           if key ~= 'L' then return false end
           local ok, err = pcall(theme.use, 'nope')
           neotmux.log.info('ok=' .. tostring(ok) .. ' err=' .. tostring(err))
           return true
         end)",
    );
    keys(&sub, &["L"]);
    let said = logged(&sub);
    assert_eq!(said.len(), 1);
    assert!(said[0].starts_with("ok=false"), "{said:?}");
    assert!(said[0].contains("unknown theme 'nope'"), "{said:?}");
}

/// A theme file that is *present* but raises on its own — a syntax error or a bug in the file
/// itself — lets that error through rather than being reported as an unknown theme.
#[test]
fn theme_use_lets_a_broken_theme_files_own_error_through() {
    let (_dir, sub) = built_with(
        "local theme = require('neotmux.theme')
         neotmux.on_key(function(key)
           if key ~= 'L' then return false end
           local ok, err = pcall(theme.use, 'broken')
           neotmux.log.info('ok=' .. tostring(ok) .. ' err=' .. tostring(err))
           return true
         end)",
        &[("lua/neotmux/themes/broken.lua", "error('boom')")],
        Rc::new(neotmux_lua::host::NoRunner),
    );
    keys(&sub, &["L"]);
    let said = logged(&sub);
    assert_eq!(said.len(), 1);
    assert!(said[0].starts_with("ok=false"), "{said:?}");
    assert!(said[0].contains("boom"), "{said:?}");
    assert!(!said[0].contains("unknown theme"), "{said:?}");
}

/// `theme.current` starts as `'catppuccin-mocha'`.
#[test]
fn theme_current_starts_as_catppuccin_mocha() {
    let (_dir, sub) = built(
        "local theme = require('neotmux.theme')
         neotmux.on_key(function(key)
           if key ~= 'L' then return false end
           neotmux.log.info('current=' .. theme.current)
           return true
         end)",
    );
    keys(&sub, &["L"]);
    assert_eq!(logged(&sub), vec!["current=catppuccin-mocha"]);
}

/// `theme` is registered as a command, so the palette has it.
#[test]
fn the_theme_command_is_registered() {
    let (_dir, sub) = built("");
    assert!(
        sub.commands().into_iter().any(|c| c.name == "theme"),
        "no theme command"
    );
}

/// The `theme` command opens a list of every shipped theme; Enter on one applies it and
/// notifies, the same way `tests/list.rs` drives a list.
#[test]
fn the_theme_command_opens_a_list_and_enter_applies_and_notifies() {
    let (_dir, sub) = built(
        "neotmux.on_key(function(key)
           if key ~= 'T' then return false end
           neotmux.command.run('theme')
           return true
         end)",
    );
    keys(&sub, &["T"]);
    sub.dispatch_tick(one());
    let cmds = sub.frame_output();
    let said: Vec<String> = cmds
        .iter()
        .filter_map(|cmd| match cmd {
            neotmux_proto::DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        said.iter().any(|t| t.contains("catppuccin-mocha")),
        "{said:?}"
    );

    keys(&sub, &["Enter"]);
    let notes = notified(&sub);
    let wants_use = "neotmux.theme').use('catppuccin-latte') to init.lua, before any theme.set";
    assert!(
        notes
            .iter()
            .any(|n| n.contains("theme catppuccin-latte") && n.contains(wants_use)),
        "{notes:?}"
    );
}

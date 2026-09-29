//! `neotmux.hl`: highlight groups the host holds, and `neotmux.style` publishing them.

use neotmux_lua::host::Op;

mod support;

use support::{Built, built};

/// Every line the test's own top-level script logged. It runs inside `run_init`, before any
/// session is bound, so its ops are filed under `None` — unlike `support::logged`, which is
/// for a dispatch bound to a session.
fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(None)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_group_set_is_the_group_got() {
    let (_dir, sub) = built(
        "neotmux.hl.set('mine:x', { fg = '#112233', bold = true })\n\
         local g = neotmux.hl.get('mine:x')\n\
         neotmux.log.info(g.fg .. ' ' .. tostring(g.bg) .. ' ' .. tostring(g.bold))\n",
    );
    assert_eq!(logged(&sub), vec!["#112233 nil true"]);
}

#[test]
fn a_group_nobody_set_is_nil() {
    let (_dir, sub) = built("neotmux.log.info(tostring(neotmux.hl.get('nobody:set')))\n");
    assert_eq!(logged(&sub), vec!["nil"]);
}

#[test]
fn a_bad_colour_raises() {
    let (_dir, sub) = built(
        "local ok = pcall(neotmux.hl.set, 'mine:x', { fg = 'red' })\n\
         neotmux.log.info(tostring(ok))\n",
    );
    assert_eq!(logged(&sub), vec!["false"]);
}

#[test]
fn a_style_default_publishes_its_parts_and_a_theme_change_republishes_them() {
    let (_dir, sub) = built(
        "local style = require('neotmux.style')\n\
         style.default('mine', { accent = 'blue' })\n\
         local before = neotmux.hl.get('mine:accent').fg\n\
         require('neotmux.theme').set({ blue = '#000001' })\n\
         neotmux.log.info(before .. ' ' .. neotmux.hl.get('mine:accent').fg)\n",
    );
    assert_eq!(logged(&sub), vec!["#89b4fa #000001"]);
}

#[test]
fn a_users_set_on_a_parent_selector_reaches_the_childs_group() {
    let (_dir, sub) = built(
        "local style = require('neotmux.style')\n\
         style.default('float.list', { frame = 'crust' })\n\
         style.set('float', { frame = '#abcdef' })\n\
         neotmux.log.info(neotmux.hl.get('float.list:frame').fg)\n",
    );
    assert_eq!(logged(&sub), vec!["#abcdef"]);
}

#[test]
fn hl_cannot_be_replaced() {
    let (_dir, sub) =
        built("neotmux.log.info(tostring(pcall(function() neotmux.hl.set = nil end)))\n");
    assert_eq!(logged(&sub), vec!["false"]);
}

#[test]
fn a_style_part_is_also_published_as_a_background_and_a_theme_change_republishes_it() {
    let (_dir, sub) = built(
        "local style = require('neotmux.style')\n\
         style.default('mine', { accent = 'blue' })\n\
         local g = neotmux.hl.get('mine:accent:bg')\n\
         local before = g.bg .. ' ' .. tostring(g.fg)\n\
         require('neotmux.theme').set({ blue = '#000001' })\n\
         neotmux.log.info(before .. ' ' .. neotmux.hl.get('mine:accent:bg').bg)\n",
    );
    assert_eq!(logged(&sub), vec!["#89b4fa nil #000001"]);
}

#[test]
fn the_sidebars_here_is_published_as_a_background_that_follows_the_theme() {
    let (_dir, sub) = built(
        "require('plugin.herdr.sidebar')\n\
         local before = neotmux.hl.get('sidebar:here:bg').bg\n\
         require('neotmux.theme').set({ base = '#000002' })\n\
         neotmux.log.info(before .. ' ' .. neotmux.hl.get('sidebar:here:bg').bg)\n",
    );
    assert_eq!(logged(&sub), vec!["#1e1e2e #000002"]);
}

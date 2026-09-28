//! `neotmux.theme` and `neotmux.style`: named colours, and the cascade every plugin reads its
//! colours from.

mod support;

use support::{built, keys, logged};

/// A key that logs `style.get(selector)[part]` for each pair, after the init has run.
fn probe(pairs: &[(&str, &str)]) -> String {
    let lines: String = pairs
        .iter()
        .map(|(selector, part)| {
            format!(
                "neotmux.log.info('{selector}.{part}=' .. tostring(require('neotmux.style').get('{selector}').{part}))\n"
            )
        })
        .collect();
    format!(
        "neotmux.on_key(function(key) if key ~= 'P' then return false end\n{lines}return true end)\n"
    )
}

#[test]
fn a_users_set_beats_a_default_whatever_order_they_ran_in() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.set('status', {{ accent = 'mauve' }})
         style.default('status', {{ accent = 'blue', bar = 'mantle' }})
         {}",
        probe(&[("status", "accent"), ("status", "bar")])
    ));
    keys(&sub, &["P"]);
    assert_eq!(
        logged(&sub),
        vec!["status.accent=#cba6f7", "status.bar=#181825"]
    );
}

#[test]
fn a_more_specific_selector_wins_and_a_users_general_one_beats_a_specific_default() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.default('float', {{ frame = 'crust', text = 'text' }})
         style.default('float.list', {{ frame = 'mantle' }})
         style.set('*', {{ text = 'red' }})
         {}",
        probe(&[
            ("float.list", "frame"),
            ("float.list", "text"),
            ("float", "frame")
        ])
    ));
    keys(&sub, &["P"]);
    assert_eq!(
        logged(&sub),
        vec![
            "float.list.frame=#181825",
            "float.list.text=#f38ba8",
            "float.frame=#11111b"
        ]
    );
}

#[test]
fn a_second_default_replaces_the_first() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.default('sample', {{ time = 'blue', rule = 'red' }})
         style.default('sample', {{ time = 'green' }})
         {}",
        probe(&[("sample", "time"), ("sample", "rule")])
    ));
    keys(&sub, &["P"]);
    assert_eq!(logged(&sub), vec!["sample.time=#a6e3a1", "sample.rule=nil"]);
}

#[test]
fn a_hex_is_itself_and_an_unknown_name_is_nil() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.set('x', {{ a = '#ff8800', b = 'no-such-colour' }})
         {}",
        probe(&[("x", "a"), ("x", "b")])
    ));
    keys(&sub, &["P"]);
    assert_eq!(logged(&sub), vec!["x.a=#ff8800", "x.b=nil"]);
}

/// The theme is read when a colour is asked for, so swapping it recolours the next frame.
#[test]
fn a_theme_change_reaches_the_next_get() {
    let (_dir, sub) = built(&format!(
        "local style = require('neotmux.style')
         style.default('status', {{ accent = 'blue' }})
         neotmux.on_key(function(key)
           if key ~= 'T' then return false end
           require('neotmux.theme').set({{ blue = '#010203' }})
           return true
         end)
         {}",
        probe(&[("status", "accent")])
    ));
    keys(&sub, &["P", "T", "P"]);
    assert_eq!(
        logged(&sub),
        vec!["status.accent=#89b4fa", "status.accent=#010203"]
    );
}
